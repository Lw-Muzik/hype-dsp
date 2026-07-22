//! The real-time audio engine.
//!
//! Pulls audio from a source, runs it through the [`hm_dsp::ProcessChain`], and
//! writes it to the default output device — applying live parameter changes and
//! emitting meter telemetry, all without allocating, locking, or doing I/O on
//! the audio callback thread.
//!
//! ## Threading
//!
//! `cpal::Stream` is `!Send` on macOS, so it is owned by a dedicated control
//! thread that never lets it cross threads. Commands ([`play`]/[`stop`]) reach
//! that thread over a channel. Parameter changes flow lock-free through an
//! [`ArcSwap`]; meters flow back through atomics. The audio callback only ever
//! *reads* params and *writes* meters — both lock-free.
//!
//! [`play`]: AudioEngine::play
//! [`stop`]: AudioEngine::stop

use std::path::Path;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hm_core::{
    BassBoostState, CompanderState, ConvolverState, EngineState, HeadphoneCorrectionState,
    MeterFrame, OutputState, ParametricBand, RoomState, SaturationState, SpatialMode,
    SpatializerState, Surround3DState, SurroundSpeakers, TrackMeta,
};
use hm_dsp::{empty_ir_slot, empty_script_slot, ChainSlots, CompanderMeter, IrSlot, PreparedIr,
    ProcessChain, ScriptSlot};

use crate::ir_loader::load_ir_samples;

use crate::capture::LoopbackCaptureSource;
use crate::decode::{decode_file, resample_stereo, DecodedAudio};
use crate::error::AudioError;
use crate::queue::QueuePlaybackSource;
use crate::stream_queue::{StreamQueueSource, StreamResolver};
use crate::sources::FilePlaybackSource;
use crate::spectrum::{Analyzer, SpectrumTap};
use crate::stems::{StemGains, StemPlaybackSource, STEM_COUNT};
use crate::streaming::{RadioStreamSource, StreamTuning};
#[cfg(target_os = "macos")]
use crate::system_tap::SystemTapSource;
use crate::{AudioSource, StreamFormat};

/// The user-facing state of system-wide EQ, so the UI can distinguish "running"
/// from "temporarily recovering" from "off". Read over IPC via a Tauri command.
///
/// Serialised as its lowercase variant name (`"active"` / `"recovering"` /
/// `"disabled"`) for the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum SystemEqStatus {
    /// Not running (never started, or turned off / superseded by normal playback).
    Disabled = 0,
    /// The tap is running and equalising system audio normally.
    Active = 1,
    /// A transient failure is being recovered from (rebuilding, or in a post-
    /// give-up cool-down before the next retry). Audio is restored but unequalised.
    Recovering = 2,
}

impl SystemEqStatus {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => SystemEqStatus::Active,
            2 => SystemEqStatus::Recovering,
            _ => SystemEqStatus::Disabled,
        }
    }
}

/// Output/capture-liveness tracker for the macOS system-tap watchdog.
///
/// The relevant callback bumps a heartbeat counter every block. This samples that
/// counter on a fixed cadence and reports how many consecutive samples it has been
/// frozen *while a tap session is meant to be running*. The watchdog treats a
/// count at/above its threshold as a stall and then decides (via
/// [`assess_tap_stall`]) whether that stall is a real device death/change (rebuild)
/// or mere CPU starvation on a live device (back off).
///
/// Pure and side-effect-free so it is unit-tested without Core Audio. Compiled on
/// every platform (the tests run on the dev host); only the macOS watchdog uses it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug)]
struct StallDetector {
    last_beat: u64,
    misses: u32,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl StallDetector {
    fn new() -> Self {
        Self {
            last_beat: 0,
            misses: 0,
        }
    }

    /// Feed the latest heartbeat and whether a tap session should be live (i.e.
    /// the tap is engaged and not paused). Returns the number of consecutive
    /// samples the heartbeat has been frozen — `0` the instant it advances, or
    /// while inactive. The caller compares this against its stall threshold.
    fn observe(&mut self, beat: u64, active: bool) -> u32 {
        if !active || beat != self.last_beat {
            // Not our concern, or output is progressing — healthy.
            self.last_beat = beat;
            self.misses = 0;
        } else {
            // Frozen heartbeat while the tap is supposed to be running.
            self.misses += 1;
        }
        self.misses
    }

    /// Re-arm: forget the current freeze so a fresh stall must accumulate a full
    /// window again. Called after the watchdog acts on a stall (rebuild or back
    /// off) so it neither spins nor immediately re-fires.
    fn reset(&mut self) {
        self.misses = 0;
    }
}

/// What the watchdog should do about a suspected tap stall. Kept as a small pure
/// enum so the decision — the crux of telling CPU starvation from device death —
/// is unit-tested without Core Audio.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TapAction {
    /// Output + capture are progressing — nothing to do.
    Healthy,
    /// A confirmed device death / default-device change, or a dead capture proc:
    /// tear the tap down and rebuild it on the current default device.
    Rebuild,
    /// A freeze on a live, unchanged device that looks like CPU starvation: keep
    /// the existing tap and wait longer rather than churn coreaudiod (which drops
    /// the EQ and can wedge the audio server under the very load that caused this).
    BackOff,
}

/// Decide what to do about a suspected tap stall. Pure so it is unit-tested
/// without Core Audio; this is the heart of the "starvation vs. death" fix.
///
/// - `output_stalled` / `capture_stalled`: the output / capture heartbeat has been
///   frozen for a full detection window while a tap session is live.
/// - `device_alive` / `device_changed`: liveness and identity of the output device
///   the tap was built against, from Core Audio.
/// - `backoff_exhausted`: we have already backed off the maximum number of times
///   for this stall, so a bounded rebuild attempt is now warranted even on an
///   ostensibly-alive device (e.g. a same-device sample-rate change kills the
///   output stream while the device still reports alive — only a rebuild recovers).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn assess_tap_stall(
    output_stalled: bool,
    capture_stalled: bool,
    device_alive: bool,
    device_changed: bool,
    backoff_exhausted: bool,
) -> TapAction {
    if !output_stalled && !capture_stalled {
        return TapAction::Healthy;
    }
    // A real device death or default-device change: the current tap can never
    // recover on its own — rebuild on the (new) default device.
    if device_changed || !device_alive {
        return TapAction::Rebuild;
    }
    // Device is alive and unchanged from here on.
    //
    // Capture frozen while the OUTPUT is still ticking proves callbacks CAN run
    // (so it isn't global CPU starvation): the tap's capture io_proc is genuinely
    // dead, and only a rebuild restarts it. This is "Chain B" — the failure that
    // otherwise leaves every app muted with a perfectly healthy output heartbeat.
    if capture_stalled && !output_stalled {
        return TapAction::Rebuild;
    }
    // Otherwise (output stalled, or BOTH stalled) on a live, unchanged device: this
    // looks like CPU starvation — a starved callback is indistinguishable from a
    // dead one by heartbeat alone. Keep the tap and wait, unless we have already
    // waited the maximum, in which case attempt one bounded rebuild anyway.
    if backoff_exhausted {
        TapAction::Rebuild
    } else {
        TapAction::BackOff
    }
}

/// Cool-down (seconds) before the watchdog re-arms a tap rebuild after a give-up,
/// growing exponentially with the number of consecutive give-ups and capped so we
/// keep retrying forever instead of silently disabling system EQ. Pure/tested.
///
/// `1 → 30s, 2 → 60s, 3 → 120s, 4 → 240s, 5+ → 300s`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn cooldown_secs(giveup_count: u32) -> u64 {
    const BASE: u64 = 30;
    const CAP: u64 = 300;
    let shift = giveup_count.saturating_sub(1).min(4);
    BASE.saturating_mul(1u64 << shift).min(CAP)
}

/// Bounds how hard the watchdog retries before giving up. Rapidly recreating the
/// tap + aggregate device can wedge `coreaudiod` (creates start returning `'nope'`
/// and even teardown stops taking effect, stranding a *muting* tap that only
/// process exit clears — the "everything stays muted until I quit the app" bug).
/// After `max_failures` consecutive failed rebuilds we stop retrying and disable
/// the tap entirely: audio keeps working (just unequalised) instead of looping.
///
/// Pure and side-effect-free so it unit-tests without Core Audio.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug)]
struct RebuildPolicy {
    failures: u32,
    max_failures: u32,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl RebuildPolicy {
    fn new(max_failures: u32) -> Self {
        Self {
            failures: 0,
            max_failures,
        }
    }

    /// A rebuild succeeded — the device is healthy again; clear the budget.
    fn on_success(&mut self) {
        self.failures = 0;
    }

    /// A rebuild failed. Returns `true` once the consecutive-failure budget is
    /// exhausted and the caller must give up (disable the tap, fail safe to
    /// unmuted), re-arming the budget so a later session starts fresh.
    fn on_failure(&mut self) -> bool {
        self.failures += 1;
        if self.failures >= self.max_failures {
            self.failures = 0;
            true
        } else {
            false
        }
    }
}

/// Lock-free meter telemetry written by the audio callback and read by the
/// UI-forwarding thread. Each `f32` is stored as its bit pattern in an atomic.
#[derive(Default)]
pub struct EngineMeters {
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    rms_l: AtomicU32,
    rms_r: AtomicU32,
}

impl EngineMeters {
    fn store(&self, frame: MeterFrame) {
        self.peak_l
            .store(frame.peak[0].to_bits(), Ordering::Relaxed);
        self.peak_r
            .store(frame.peak[1].to_bits(), Ordering::Relaxed);
        self.rms_l.store(frame.rms[0].to_bits(), Ordering::Relaxed);
        self.rms_r.store(frame.rms[1].to_bits(), Ordering::Relaxed);
    }

    /// Read the latest meter frame.
    pub fn load(&self) -> MeterFrame {
        MeterFrame {
            peak: [
                f32::from_bits(self.peak_l.load(Ordering::Relaxed)),
                f32::from_bits(self.peak_r.load(Ordering::Relaxed)),
            ],
            rms: [
                f32::from_bits(self.rms_l.load(Ordering::Relaxed)),
                f32::from_bits(self.rms_r.load(Ordering::Relaxed)),
            ],
        }
    }

    fn zero(&self) {
        self.store(MeterFrame::default());
    }
}

fn compute_meters(buffer: &[f32], channels: usize) -> MeterFrame {
    if channels == 0 {
        return MeterFrame::default();
    }
    let frames = buffer.len() / channels;
    if frames == 0 {
        return MeterFrame::default();
    }
    let probe = channels.min(2);
    let mut peak = [0.0f32; 2];
    let mut sumsq = [0.0f64; 2];
    for f in 0..frames {
        for ch in 0..probe {
            let v = buffer[f * channels + ch];
            let a = v.abs();
            if a > peak[ch] {
                peak[ch] = a;
            }
            sumsq[ch] += (v as f64) * (v as f64);
        }
    }
    let mut rms = [
        (sumsq[0] / frames as f64).sqrt() as f32,
        (sumsq[1] / frames as f64).sqrt() as f32,
    ];
    // Mono: mirror the single channel to both meters.
    if probe == 1 {
        peak[1] = peak[0];
        rms[1] = rms[0];
    }
    MeterFrame { peak, rms }
}

/// Lock-free transport position, shared with the UI-forwarding thread. The
/// audio callback writes position/total; a seek request flows back in.
pub struct PlaybackPos {
    position_frames: AtomicU64,
    total_frames: AtomicU64,
    sample_rate: AtomicU32,
    /// Pending seek target in frames, or `-1` for none.
    seek_to: AtomicI64,
    /// Whether the active source can be scrubbed (false for live radio).
    seekable: AtomicBool,
    /// Whether the active source is currently buffering (waiting for network).
    buffering: AtomicBool,
    /// Latest download throughput estimate from the active source, bytes/sec.
    download_bps: AtomicU64,
    /// Mid-track rebuffer event count from the active source.
    rebuffer_count: AtomicU32,
}

impl Default for PlaybackPos {
    fn default() -> Self {
        Self {
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(0),
            sample_rate: AtomicU32::new(0),
            seek_to: AtomicI64::new(-1),
            seekable: AtomicBool::new(false),
            buffering: AtomicBool::new(false),
            download_bps: AtomicU64::new(0),
            rebuffer_count: AtomicU32::new(0),
        }
    }
}

impl PlaybackPos {
    fn take_seek(&self) -> Option<usize> {
        let v = self.seek_to.swap(-1, Ordering::Relaxed);
        (v >= 0).then_some(v as usize)
    }

    fn request_seek(&self, frame: usize) {
        self.seek_to.store(frame as i64, Ordering::Relaxed);
    }

    fn write(&self, position: usize, total: usize) {
        self.position_frames
            .store(position as u64, Ordering::Relaxed);
        self.total_frames.store(total as u64, Ordering::Relaxed);
    }

    fn prepare(&self, sample_rate: u32, total: usize) {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.total_frames.store(total as u64, Ordering::Relaxed);
        self.position_frames.store(0, Ordering::Relaxed);
        self.seek_to.store(-1, Ordering::Relaxed);
        // Known length up front (files/queues) ⇒ seekable immediately; streams
        // start non-seekable and flip once the renderer learns their length.
        self.seekable.store(total > 0, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.write(0, 0);
        self.seek_to.store(-1, Ordering::Relaxed);
    }

    /// Current position in seconds.
    pub fn position_secs(&self) -> f64 {
        let rate = self.sample_rate.load(Ordering::Relaxed);
        if rate == 0 {
            return 0.0;
        }
        self.position_frames.load(Ordering::Relaxed) as f64 / rate as f64
    }

    /// Total duration in seconds, if known.
    pub fn duration_secs(&self) -> Option<f64> {
        let rate = self.sample_rate.load(Ordering::Relaxed);
        let total = self.total_frames.load(Ordering::Relaxed);
        (rate > 0 && total > 0).then(|| total as f64 / rate as f64)
    }

    /// Whether the active source can be scrubbed.
    pub fn is_seekable(&self) -> bool {
        self.seekable.load(Ordering::Relaxed)
    }

    fn set_seekable(&self, seekable: bool) {
        self.seekable.store(seekable, Ordering::Relaxed);
    }

    /// Write network-stream telemetry (buffering state + throughput + rebuffer
    /// count) from the audio callback. All stores are `Relaxed` — no ordering
    /// guarantee needed; the UI-forwarding thread polls these once per frame.
    fn write_net(&self, buffering: bool, download_bps: u64, rebuffer_count: u32) {
        self.buffering.store(buffering, Ordering::Relaxed);
        self.download_bps.store(download_bps, Ordering::Relaxed);
        self.rebuffer_count.store(rebuffer_count, Ordering::Relaxed);
    }

    /// Whether the active source is currently buffering.
    pub fn is_buffering(&self) -> bool {
        self.buffering.load(Ordering::Relaxed)
    }
    /// Latest download throughput estimate, bytes/sec (0 if not streaming).
    pub fn download_bps(&self) -> u64 {
        self.download_bps.load(Ordering::Relaxed)
    }
    /// Mid-track rebuffer event count (0 if not streaming).
    pub fn rebuffer_count(&self) -> u32 {
        self.rebuffer_count.load(Ordering::Relaxed)
    }

    /// Request a seek to `secs` (applied on the next audio block).
    pub fn seek_secs(&self, secs: f64) {
        let rate = self.sample_rate.load(Ordering::Relaxed);
        if rate > 0 {
            self.request_seek((secs.max(0.0) * rate as f64).round() as usize);
        }
    }
}

/// Owns the DSP chain and the active source; renders one block at a time.
/// Extracted from the cpal callback so it can be unit-tested without a device.
pub struct Renderer {
    chain: ProcessChain,
    source: Box<dyn AudioSource>,
    analyzer: Analyzer,
}

impl Renderer {
    /// Create a renderer for the given source and output format.
    pub fn new(
        mut source: Box<dyn AudioSource>,
        sample_rate: f32,
        channels: usize,
        slots: ChainSlots,
    ) -> Self {
        // Tell the source the real output format so rate-dependent sources (e.g.
        // the system tap) can configure their resampler. Errors are non-fatal.
        let _ = source.start(StreamFormat {
            sample_rate: sample_rate as u32,
            channels: channels as u16,
        });
        Self {
            chain: ProcessChain::standard_with_slots(sample_rate, channels, slots),
            source,
            analyzer: Analyzer::new(sample_rate),
        }
    }

    /// Fill `out` with the next processed block, updating meters and the
    /// spectrum from the post-processing output. Returns `true` when the source
    /// is fully exhausted (the block contained no source audio).
    ///
    /// Real-time safe: no allocation, locking, or I/O.
    pub fn render(
        &mut self,
        out: &mut [f32],
        channels: usize,
        state: &EngineState,
        meters: &EngineMeters,
        spectrum: &SpectrumTap,
        pos: &PlaybackPos,
    ) -> bool {
        if let Some(frame) = pos.take_seek() {
            self.source.seek(frame);
        }
        let produced = self.source.read(out, channels);
        pos.write(self.source.position(), self.source.total_frames());
        pos.set_seekable(self.source.seekable());
        pos.write_net(
            self.source.buffering(),
            self.source.download_bps(),
            self.source.rebuffer_count(),
        );

        if (state.master_volume - 1.0).abs() > f32::EPSILON {
            for s in out.iter_mut() {
                *s *= state.master_volume;
            }
        }

        if state.power {
            // Cheap when unchanged: each processor guards its own re-tuning.
            self.chain.set_params(state);
            self.chain.process(out, channels);
        }

        meters.store(compute_meters(out, channels));
        self.analyzer.push(out, channels, spectrum);
        // Live sources (radio) never signal EOF on underflow.
        produced == 0 && !self.source.is_live()
    }
}

/// Control messages to the engine thread.
enum EngineCommand {
    Play(DecodedAudio),
    /// Stream a URL (radio, or a cloud/phone file) with optional HTTP headers
    /// and an optional duration hint (seconds) when the length is known up front.
    PlayStream {
        url: String,
        headers: Vec<(String, String)>,
        duration_hint: Option<f64>,
    },
    /// Play a list of file paths gaplessly (decoded on a worker), starting at
    /// `start`. The crossfade duration is read live from the engine's shared
    /// crossfade value, so slider changes apply to the current queue.
    PlayQueue {
        paths: Vec<String>,
        start: usize,
    },
    /// Play a queue of **streamed** tracks (cloud/phone) gaplessly / crossfading.
    /// Each track's URL is resolved lazily via `resolver(index)`; only the
    /// current + next track are buffered. `count` is the total track count.
    PlayStreamQueue {
        resolver: StreamResolver,
        count: usize,
        start: usize,
    },
    PlayCapture,
    /// Play four separated stems together, mixed live by the engine's stem gains,
    /// starting at `start_secs` (so we can swap in stems at the live playhead
    /// without interrupting the track). Boxed — four decoded buffers are large.
    PlayStems {
        stems: Box<[DecodedAudio; STEM_COUNT]>,
        start_secs: f64,
    },
    /// Play an already-constructed live source (e.g. the macOS system tap).
    /// Only constructed on the macOS tap path; dead (but compiled) elsewhere.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    PlaySource(Box<dyn AudioSource>),
    /// Watchdog-triggered: the system-tap output stalled (its stream died, e.g.
    /// after a default-output-device change). Rebuild a fresh tap on the current
    /// default device so the system isn't left muted. macOS-only.
    #[cfg(target_os = "macos")]
    RestartSystemTap,
    Pause,
    Resume,
    Stop,
    Shutdown,
}

/// Handle to the audio engine. `Send + Sync`, suitable for Tauri managed state.
pub struct AudioEngine {
    /// Authoritative state for writes (serializes read-modify-write).
    write_state: Mutex<EngineState>,
    /// Lock-free snapshot the audio callback reads each block.
    shared: Arc<ArcSwap<EngineState>>,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    /// Current track's now-playing metadata (tags + cover), for the UI.
    track_meta: Arc<ArcSwap<TrackMeta>>,
    /// Bumped whenever `track_meta` changes, so observers can detect new tracks.
    meta_version: Arc<AtomicU64>,
    /// Absolute queue index the engine is currently playing (gapless/crossfade).
    queue_index: Arc<AtomicUsize>,
    /// Live crossfade duration in seconds (f32 bits), read by the active queue
    /// each block so slider changes apply to the current queue immediately.
    crossfade: Arc<AtomicU32>,
    /// Live per-stem gains, shared with the active stem-playback source so the
    /// UI's faders apply to the current track instantly.
    stem_gains: Arc<StemGains>,
    /// Lock-free IR slot shared with the active Convolver; the UI publishes a
    /// freshly-built `PreparedIr` here and the audio thread reads it next block.
    ir_slot: IrSlot,
    /// Lock-free per-band GR meter shared with the active Compander; written on
    /// the audio thread once per block and read by the UI-forwarding thread.
    compander_gr: std::sync::Arc<CompanderMeter>,
    /// Lock-free slot holding the compiled LiveProg program, shared with the
    /// chain's script stage. Published on compile; the audio thread only loads.
    script_slot: ScriptSlot,
    /// Keeps the macOS tap watchdog thread alive; cleared on drop so it exits.
    /// (The heartbeat and tap-active flags it watches live only as clones handed
    /// to the control + watchdog threads — they need no handle on the engine.)
    #[cfg(target_os = "macos")]
    watchdog_alive: Arc<AtomicBool>,
    /// Proactive default-output-device listener: rebuilds the tap on the new
    /// device the instant macOS switches output, so recovery doesn't wait for the
    /// watchdog's stall window. Held only for its `Drop` (which unregisters it).
    #[cfg(target_os = "macos")]
    _device_listener: Option<crate::system_tap::DefaultOutputListener>,
    /// Shared capture telemetry for the macOS tap: the io_proc bumps its heartbeat
    /// (RT-safe) and the watchdog watches it. Held here so `play_system_tap` can
    /// build the tap against the same instance the watchdog/control thread share.
    #[cfg(target_os = "macos")]
    capture_tel: Arc<crate::system_tap::CaptureTelemetry>,
    /// Current system-wide-EQ status (`Active`/`Recovering`/`Disabled`), readable
    /// over IPC so the UI can reflect recovery instead of a silent stall. Written
    /// by the control + watchdog threads; read by [`AudioEngine::system_eq_status`].
    system_eq_status: Arc<AtomicU8>,
    ctrl: Mutex<mpsc::Sender<EngineCommand>>,
    /// Active self-contained system-wide EQ pipeline (Linux/Windows re-routing).
    /// Held only to keep it running; dropping it tears the routing down. `Send`-
    /// boxed since the concrete type is platform-specific.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    system_eq: Mutex<Option<Box<dyn Send>>>,
    _thread: JoinHandle<()>,
}

/// Metadata about a freshly-loaded impulse response, returned to the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvolverIrInfo {
    pub name: String,
    pub seconds: f32,
    pub truncated: bool,
    pub channels: usize,
}

/// A write handle to the engine's now-playing metadata slot, handed to a stream
/// decode thread so it can publish tags + cover art once it has probed.
#[derive(Clone)]
pub struct MetaSink {
    meta: Arc<ArcSwap<TrackMeta>>,
    version: Arc<AtomicU64>,
}

impl MetaSink {
    /// Publish freshly-extracted metadata for the current track.
    pub fn set(&self, meta: TrackMeta) {
        self.meta.store(Arc::new(meta));
        self.version.fetch_add(1, Ordering::Release);
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// Spawn the engine's control thread and return a handle to it.
    pub fn new() -> Self {
        let initial = EngineState::default();
        let shared = Arc::new(ArcSwap::from_pointee(initial.clone()));
        let meters = Arc::new(EngineMeters::default());
        let spectrum = Arc::new(SpectrumTap::default());
        let pos = Arc::new(PlaybackPos::default());
        let playing = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let track_meta = Arc::new(ArcSwap::from_pointee(TrackMeta::default()));
        let meta_version = Arc::new(AtomicU64::new(0));
        let queue_index = Arc::new(AtomicUsize::new(0));
        let crossfade = Arc::new(AtomicU32::new(initial.playback.crossfade_secs.to_bits()));
        let stem_gains = Arc::new(StemGains::default());
        let ir_slot = empty_ir_slot();
        let compander_gr = std::sync::Arc::new(CompanderMeter::default());
        let script_slot = empty_script_slot();
        let chain_slots = ChainSlots {
            ir: ir_slot.clone(),
            compander_meter: compander_gr.clone(),
            script: script_slot.clone(),
        };
        let output_beat = Arc::new(AtomicU64::new(0));
        let tap_active = Arc::new(AtomicBool::new(false));
        let tap_rebuild_pending = Arc::new(AtomicBool::new(false));
        let system_eq_status = Arc::new(AtomicU8::new(SystemEqStatus::Disabled as u8));
        // macOS tap-recovery state (see `tap_watchdog` / `RestartSystemTap`).
        #[cfg(target_os = "macos")]
        let capture_tel = Arc::new(crate::system_tap::CaptureTelemetry::default());
        #[cfg(target_os = "macos")]
        let tap_output_device_id = Arc::new(AtomicU32::new(0));
        #[cfg(target_os = "macos")]
        let tap_gave_up = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "macos")]
        let device_change_signal = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        let thread = {
            let shared = shared.clone();
            let meters = meters.clone();
            let spectrum = spectrum.clone();
            let pos = pos.clone();
            let playing = playing.clone();
            let paused = paused.clone();
            let track_meta = track_meta.clone();
            let meta_version = meta_version.clone();
            let queue_index = queue_index.clone();
            let crossfade = crossfade.clone();
            let stem_gains = stem_gains.clone();
            let chain_slots = chain_slots.clone();
            let output_beat = output_beat.clone();
            let tap_active = tap_active.clone();
            let tap_rebuild_pending = tap_rebuild_pending.clone();
            #[cfg(target_os = "macos")]
            let system_eq_status = system_eq_status.clone();
            #[cfg(target_os = "macos")]
            let capture_tel = capture_tel.clone();
            #[cfg(target_os = "macos")]
            let tap_output_device_id = tap_output_device_id.clone();
            #[cfg(target_os = "macos")]
            let tap_gave_up = tap_gave_up.clone();
            std::thread::Builder::new()
                .name("hm-audio-engine".into())
                .spawn(move || {
                    control_loop(ControlCtx {
                        rx,
                        shared,
                        meters,
                        spectrum,
                        pos,
                        playing,
                        paused,
                        track_meta,
                        meta_version,
                        queue_index,
                        crossfade,
                        stem_gains,
                        chain_slots,
                        output_beat,
                        tap_active,
                        tap_rebuild_pending,
                        #[cfg(target_os = "macos")]
                        system_eq_status,
                        #[cfg(target_os = "macos")]
                        capture_tel,
                        #[cfg(target_os = "macos")]
                        tap_output_device_id,
                        #[cfg(target_os = "macos")]
                        tap_gave_up,
                    })
                })
                .expect("failed to spawn hm-audio engine thread")
        };

        // macOS system-tap watchdog: because the tap mutes every other app, a
        // dead output stream (e.g. a default-output-device change) would leave the
        // whole system silent. This thread notices the stall and asks the control
        // thread to rebuild the tap on the current default device.
        #[cfg(target_os = "macos")]
        let watchdog_alive = Arc::new(AtomicBool::new(true));
        #[cfg(target_os = "macos")]
        {
            std::thread::Builder::new()
                .name("hm-audio-tap-watchdog".into())
                .spawn({
                    let watch = TapWatch {
                        alive: watchdog_alive.clone(),
                        output_beat: output_beat.clone(),
                        capture_tel: capture_tel.clone(),
                        tap_active: tap_active.clone(),
                        paused: paused.clone(),
                        rebuild_pending: tap_rebuild_pending.clone(),
                        tap_output_device_id: tap_output_device_id.clone(),
                        tap_gave_up: tap_gave_up.clone(),
                        device_change_signal: device_change_signal.clone(),
                        status: system_eq_status.clone(),
                        ctrl: tx.clone(),
                    };
                    move || tap_watchdog(watch)
                })
                .expect("failed to spawn hm-audio tap watchdog");
        }

        // Proactive recovery: the instant macOS switches the default output device,
        // flag it so the watchdog rebuilds the tap on the new device on its next
        // tick. The listener callback runs on a Core Audio thread and must be
        // allocation-free (an mpsc send allocates and could abort across the
        // `C-unwind` boundary under memory pressure), so it ONLY flips an atomic;
        // the watchdog thread performs the channel send. `None` if registration
        // fails — the watchdog's stall detection backstops it.
        #[cfg(target_os = "macos")]
        let device_listener = {
            let signal = device_change_signal.clone();
            crate::system_tap::DefaultOutputListener::new(Box::new(move || {
                signal.store(true, Ordering::Relaxed);
            }))
            .ok()
        };

        Self {
            write_state: Mutex::new(initial),
            shared,
            meters,
            spectrum,
            pos,
            playing,
            paused,
            track_meta,
            meta_version,
            queue_index,
            crossfade,
            stem_gains,
            ir_slot,
            compander_gr,
            script_slot,
            #[cfg(target_os = "macos")]
            watchdog_alive,
            #[cfg(target_os = "macos")]
            _device_listener: device_listener,
            #[cfg(target_os = "macos")]
            capture_tel,
            system_eq_status,
            ctrl: Mutex::new(tx),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            system_eq: Mutex::new(None),
            _thread: thread,
        }
    }

    /// Shared handles to the now-playing metadata + its version counter, for the
    /// frame-forwarder to emit `engine:now_playing` when a new track starts.
    pub fn track_meta_handle(&self) -> Arc<ArcSwap<TrackMeta>> {
        self.track_meta.clone()
    }
    pub fn meta_version_handle(&self) -> Arc<AtomicU64> {
        self.meta_version.clone()
    }
    /// The absolute queue index currently playing, for the forwarder to emit.
    pub fn queue_index_handle(&self) -> Arc<AtomicUsize> {
        self.queue_index.clone()
    }

    /// Current engine state.
    pub fn state(&self) -> EngineState {
        self.write_state
            .lock()
            .expect("engine state poisoned")
            .clone()
    }

    /// A shared handle to the live state snapshot, for off-thread observers
    /// (e.g. an autosave loop that persists settings when they change).
    pub fn state_handle(&self) -> Arc<ArcSwap<EngineState>> {
        self.shared.clone()
    }

    /// Current system-wide-EQ status (`Active` / `Recovering` / `Disabled`).
    ///
    /// On macOS this reflects the tap watchdog's live view — notably `Recovering`
    /// while a stall is being rebuilt or is in a post-give-up cool-down, so the UI
    /// can show "recovering…" instead of appearing to have silently died.
    pub fn system_eq_status(&self) -> SystemEqStatus {
        SystemEqStatus::from_u8(self.system_eq_status.load(Ordering::Relaxed))
    }

    fn update(&self, f: impl FnOnce(&mut EngineState)) {
        let mut guard = self.write_state.lock().expect("engine state poisoned");
        f(&mut guard);
        self.shared.store(Arc::new(guard.clone()));
    }

    /// Toggle the global enhancement power (chain bypass).
    pub fn set_power(&self, on: bool) {
        self.update(|s| s.power = on);
    }

    /// Set the master output volume (linear, clamped to a safe range).
    pub fn set_master_volume(&self, volume: f32) {
        self.update(|s| s.master_volume = volume.clamp(0.0, 4.0));
    }

    /// Replace the full engine state (used on startup to restore saved settings).
    pub fn set_state(&self, new_state: EngineState) {
        // Keep the live crossfade value in sync with the restored state, so a
        // saved crossfade takes effect immediately (it's read off the atomic).
        self.crossfade
            .store(new_state.playback.crossfade_secs.max(0.0).to_bits(), Ordering::Relaxed);
        // Same reasoning, one step further: only the script's *source* is
        // serializable, so a restored or preset-applied state carries text with
        // no program behind it. Without this the card would show an enabled
        // script, with the user's code in it, over a chain running identity.
        self.recompile_from_state(&new_state);
        let mut guard = self.write_state.lock().expect("engine state poisoned");
        *guard = new_state.clone();
        self.shared.store(Arc::new(new_state));
    }

    /// Apply a manual EQ edit. Clears the active preset (now customized).
    pub fn set_eq(&self, bands: [f32; hm_core::BAND_COUNT], pre_gain: f32, enabled: bool) {
        self.update(|s| {
            s.eq.bands = bands;
            s.eq.pre_gain = pre_gain;
            s.eq.enabled = enabled;
            s.active_preset_id = None;
        });
    }

    /// Apply a named preset's curve and mark it active.
    pub fn apply_eq_preset(
        &self,
        bands: [f32; hm_core::BAND_COUNT],
        pre_gain: f32,
        preset_id: String,
    ) {
        self.update(|s| {
            s.eq.bands = bands;
            s.eq.pre_gain = pre_gain;
            s.active_preset_id = Some(preset_id);
        });
    }

    /// Configure the bass boost stage.
    pub fn set_bass(&self, enabled: bool, amount: f32, harmonics: bool, adaptive: bool) {
        self.update(|s| {
            s.bass = BassBoostState {
                enabled,
                amount,
                harmonics,
                adaptive,
            };
        });
    }

    /// Update queue-playback behaviour (gapless + crossfade). Takes effect on the
    /// next queue played.
    pub fn set_playback(&self, gapless: bool, crossfade_secs: f32) {
        let crossfade = crossfade_secs.max(0.0);
        // Persist in state (for autosave/restore)...
        self.update(|s| {
            s.playback = hm_core::PlaybackState { gapless, crossfade_secs: crossfade, data_saver: s.playback.data_saver, autoplay: s.playback.autoplay };
        });
        // ...and publish to the live value the active queue reads each block, so
        // the change takes effect on the current queue's next transition.
        self.crossfade.store(crossfade.to_bits(), Ordering::Relaxed);
    }

    /// Toggle Data Saver (low-bandwidth) mode. Takes effect on the next stream.
    pub fn set_data_saver(&self, on: bool) {
        self.update(|s| s.playback.data_saver = on);
    }

    /// Toggle Autoplay (endless queue extension). The engine never reads this —
    /// the frontend queue does — it lives here to ride the state autosave.
    pub fn set_autoplay(&self, on: bool) {
        self.update(|s| s.playback.autoplay = on);
    }

    /// The current crossfade duration (seconds); 0 means gapless-only.
    pub fn crossfade_secs(&self) -> f32 {
        f32::from_bits(self.crossfade.load(Ordering::Relaxed))
    }

    /// Configure the spatializer stage.
    pub fn set_spatializer(&self, enabled: bool, amount: f32, mode: SpatialMode) {
        self.update(|s| {
            s.spatializer = SpatializerState {
                enabled,
                amount: amount.clamp(0.0, 1.0),
                mode,
            };
        });
    }

    /// Configure the 3D-surround (virtual-speaker) stage.
    pub fn set_surround3d(
        &self,
        enabled: bool,
        intensity: f32,
        subwoofer: f32,
        speakers: SurroundSpeakers,
    ) {
        self.update(|s| {
            s.surround3d = Surround3DState {
                enabled,
                intensity: intensity.clamp(0.0, 1.0),
                subwoofer: subwoofer.clamp(0.0, 1.0),
                speakers,
            };
        });
    }

    /// Configure the room-reverb stage (clamps all params to range).
    pub fn set_room(&self, mut room: RoomState) {
        room.room_size = room.room_size.clamp(0.0, 1.0);
        room.decay = room.decay.clamp(0.0, 1.0);
        room.damping = room.damping.clamp(0.0, 1.0);
        room.pre_delay = room.pre_delay.clamp(0.0, 200.0);
        room.diffusion = room.diffusion.clamp(0.0, 1.0);
        room.wet_dry = room.wet_dry.clamp(0.0, 1.0);
        self.update(|s| s.room = room);
    }

    /// Configure the multiband compander stage.
    pub fn set_compander(&self, mut compander: CompanderState) {
        compander.ratio = compander.ratio.max(1.0);
        compander.expander_ratio = compander.expander_ratio.max(1.0);
        compander.knee_db = compander.knee_db.max(0.0);
        compander.attack_ms = compander.attack_ms.max(0.1);
        compander.release_ms = compander.release_ms.max(0.1);
        self.update(|s| s.compander = compander);
    }

    /// Compile a LiveProg source and publish it to the chain's script stage.
    ///
    /// Compilation happens on whichever thread calls this. The only guarantee
    /// that matters is that it is never the audio thread: that thread's entire
    /// involvement with a program is one atomic load per block.
    ///
    /// On success the source is stored in engine state so it persists and is
    /// captured by whole-chain presets. On failure nothing is published and the
    /// previously-running program keeps playing — a script that no longer
    /// compiles is a reason to leave the sound alone, not to silence it.
    pub fn compile_script(&self, source: String) -> Result<(), hm_dsp::script::ScriptError> {
        let program = hm_dsp::script::compile(&source)?;
        self.script_slot
            .store(std::sync::Arc::new(Some(std::sync::Arc::new(program))));
        self.update(|s| s.script.source = source);
        Ok(())
    }

    /// Enable or disable the script stage without recompiling.
    pub fn set_script(&self, enabled: bool) {
        self.update(|s| s.script.enabled = enabled);
    }

    /// Rebuild the slot from a state that was restored rather than authored.
    fn recompile_from_state(&self, state: &EngineState) {
        publish_script(&self.script_slot, &state.script.source);
    }

    /// Configure the tube saturation stage.
    pub fn set_saturation(&self, mut saturation: SaturationState) {
        saturation.drive = saturation.drive.clamp(0.0, 1.0);
        saturation.mix = saturation.mix.clamp(0.0, 1.0);
        self.update(|s| s.saturation = saturation);
    }

    /// Configure the output stage: makeup gain and the brickwall limiter.
    ///
    /// The limiter is on by default and is the clip-safety net for boosted
    /// audio; disabling it (`limiter_enabled = false`) is a deliberate user
    /// choice that lets peaks pass through unlimited, so it can clip.
    pub fn set_output(&self, mut output: OutputState) {
        output.gain_db = output.gain_db.clamp(-24.0, 24.0);
        output.ceiling_db = output.ceiling_db.clamp(-24.0, 0.0);
        self.update(|s| s.output = output);
    }

    /// Update the convolver's cheap scalar params (enabled / wet-dry / gain).
    /// Does NOT reload or rebuild the IR — just merges scalars while preserving
    /// any already-loaded IR metadata published by `load_convolver_ir`.
    pub fn set_convolver(&self, mut state: ConvolverState) {
        state.wet_dry = state.wet_dry.clamp(0.0, 1.0);
        self.update(|s| {
            // Preserve loaded-IR metadata published by load_convolver_ir.
            let (id, name, secs, trunc) = (
                s.convolver.ir_id.clone(),
                s.convolver.ir_name.clone(),
                s.convolver.ir_seconds,
                s.convolver.ir_truncated,
            );
            s.convolver = ConvolverState {
                ir_id: state.ir_id.or(id),
                ir_name: state.ir_name.or(name),
                ir_seconds: if state.ir_seconds > 0.0 { state.ir_seconds } else { secs },
                // ir_truncated is sticky: cleared only by a fresh load_convolver_ir, not by knob changes.
                ir_truncated: state.ir_truncated || trunc,
                ..state
            };
        });
    }

    /// Decode, prepare, and publish an impulse response to the live stage.
    ///
    /// Heavy work (file I/O + FFT/resample) runs here — on the caller's command
    /// thread, never on the audio thread.  The prepared IR is handed off to the
    /// audio thread by a single lock-free atomic store; the audio callback only
    /// ever calls `slot.load()`.
    pub fn load_convolver_ir(&self, path: &Path) -> Result<ConvolverIrInfo, AudioError> {
        // TODO: thread real device sample rate if it becomes available.
        let target_sr = 48_000.0_f32;
        let (samples, channels, src_sr) = load_ir_samples(path)?;
        let prepared = PreparedIr::build(&samples, channels, src_sr, target_sr);
        let info = ConvolverIrInfo {
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("IR")
                .to_string(),
            seconds: prepared.seconds,
            truncated: prepared.truncated,
            channels: prepared.channels,
        };
        // Publish IR to the audio thread (lock-free) BEFORE updating state, so the audio path never references an IR not yet in the slot.
        self.ir_slot
            .store(Arc::new(Some(Arc::new(prepared))));
        // Reflect metadata in state so the UI + autosave see it.
        let info_c = info.clone();
        let id = path.to_string_lossy().to_string();
        self.update(|s| {
            s.convolver.ir_id = Some(id);
            s.convolver.ir_name = Some(info_c.name.clone());
            s.convolver.ir_seconds = info_c.seconds;
            s.convolver.ir_truncated = info_c.truncated;
            // Loading an IR implicitly enables the convolver.
            s.convolver.enabled = true;
        });
        Ok(info)
    }

    /// Load a headphone profile's correction into the chain and mark it active.
    pub fn set_headphone(&self, bands: Vec<ParametricBand>, preamp: f32, profile_id: String) {
        self.update(|s| {
            s.headphone = HeadphoneCorrectionState {
                enabled: true,
                preamp,
                bands,
            };
            s.active_profile_id = Some(profile_id);
        });
    }

    /// Clear any active headphone correction.
    pub fn clear_headphone(&self) {
        self.update(|s| {
            s.headphone = HeadphoneCorrectionState::default();
            s.active_profile_id = None;
        });
    }

    /// Shared meter telemetry for the UI-forwarding thread.
    pub fn meters(&self) -> Arc<EngineMeters> {
        self.meters.clone()
    }

    /// Shared spectrum telemetry for the UI-forwarding thread.
    pub fn spectrum(&self) -> Arc<SpectrumTap> {
        self.spectrum.clone()
    }

    /// Shared per-band gain-reduction meter for the UI-forwarding thread.
    pub fn compander_gr(&self) -> std::sync::Arc<CompanderMeter> {
        self.compander_gr.clone()
    }

    /// Shared transport position for the UI-forwarding thread.
    pub fn pos(&self) -> Arc<PlaybackPos> {
        self.pos.clone()
    }

    /// Shared paused flag for the UI-forwarding thread.
    pub fn paused_flag(&self) -> Arc<AtomicBool> {
        self.paused.clone()
    }

    /// Whether playback is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Seek to `secs` within the current track.
    pub fn seek(&self, secs: f64) {
        self.pos.seek_secs(secs);
    }

    /// Pause playback (keeps position).
    pub fn pause(&self) {
        let _ = self
            .ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::Pause);
    }

    /// Resume playback.
    pub fn resume(&self) {
        let _ = self
            .ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::Resume);
    }

    /// Shared playing flag for the UI-forwarding thread.
    pub fn playing_flag(&self) -> Arc<AtomicBool> {
        self.playing.clone()
    }

    /// Whether audio is currently playing.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Decode `path` (off the audio thread) and start playback through the
    /// chain. Decode errors are returned synchronously.
    pub fn play_file(&self, path: &Path) -> Result<(), AudioError> {
        let audio = decode_file(path)?;
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::Play(audio))
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Play four separated stems (decoded; any rate) together, mixed live by the
    /// shared [`StemGains`], beginning at `start_secs`. Each is resampled to the
    /// device rate on the engine thread; the DSP chain still applies to the mix.
    ///
    /// Passing the current transport position as `start_secs` lets the caller
    /// swap stems in seamlessly while a track plays: at unity gain the stems sum
    /// back to the original mix, so playback continues without a gap.
    pub fn play_stems(
        &self,
        stems: [DecodedAudio; STEM_COUNT],
        start_secs: f64,
    ) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayStems {
                stems: Box::new(stems),
                start_secs,
            })
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Set a stem's gain (0 = muted, 1 = unity), applied live + smoothed.
    pub fn set_stem_gain(&self, stem: usize, gain: f32) {
        self.stem_gains.set(stem, gain);
    }

    /// The shared stem gains (e.g. to read current fader positions).
    pub fn stem_gains(&self) -> Arc<StemGains> {
        self.stem_gains.clone()
    }

    /// Stream and play an internet radio URL through the chain.
    pub fn play_radio(&self, url: String) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayStream {
                url,
                headers: Vec::new(),
                duration_hint: None,
            })
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Stream and play a URL with extra HTTP headers (e.g. a cloud file that
    /// needs an `Authorization: Bearer …`), through the chain. `duration_hint`
    /// (seconds) makes the stream seekable when the container omits a length.
    pub fn play_stream(
        &self,
        url: String,
        headers: Vec<(String, String)>,
        duration_hint: Option<f64>,
    ) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayStream {
                url,
                headers,
                duration_hint,
            })
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Play a list of file paths as a gapless (and optionally crossfading) queue,
    /// starting at `start`. Tracks decode on a background worker. The crossfade
    /// duration is read live from `set_playback`, so it can change mid-queue.
    pub fn play_queue(&self, paths: Vec<String>, start: usize) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayQueue { paths, start })
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Play a queue of streamed tracks (cloud/phone) gaplessly / crossfading.
    /// `resolver(i)` resolves track `i`'s URL lazily; only the current + next
    /// track are streamed/decoded, so a long queue stays memory-bounded.
    pub fn play_stream_queue(
        &self,
        resolver: StreamResolver,
        count: usize,
        start: usize,
    ) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayStreamQueue {
                resolver,
                count,
                start,
            })
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Capture the default input device through the chain (driver-free stand-in).
    pub fn play_capture(&self) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayCapture)
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Equalize **system-wide** audio via a Core Audio process tap (macOS 14.4+).
    /// Creating the tap triggers the audio-capture permission prompt and returns
    /// its error synchronously (e.g. permission denied).
    #[cfg(target_os = "macos")]
    pub fn play_system_tap(&self) -> Result<(), AudioError> {
        // The tap is scoped by the current per-app selection (`system_eq_scope`).
        let scope = self.state().system_eq_scope;
        let source = SystemTapSource::new(48_000, self.capture_tel.clone(), &scope)?;
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlaySource(Box::new(source)))
            .map_err(|_| AudioError::Stream("engine thread stopped".into()))
    }

    /// Set which apps the system-wide EQ tap processes. Cheap state update; the
    /// caller restarts the tap (via `play_system_tap`) for it to take effect.
    pub fn set_system_eq_scope(&self, scope: hm_core::SystemEqScope) {
        self.update(|s| s.system_eq_scope = scope);
    }

    /// Start the self-contained system-wide EQ pipeline (Linux/Windows): re-route
    /// every app's audio through the DSP chain by capturing a virtual device and
    /// rendering the processed result to the real output. Idempotent — any
    /// running pipeline is torn down first. The macOS path uses `play_system_tap`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub fn start_system_eq(&self) -> Result<(), AudioError> {
        // Tear down any existing pipeline before starting a fresh one.
        self.stop_system_eq();
        let state = self.shared.clone();
        #[cfg(target_os = "linux")]
        let session: Box<dyn Send> =
            Box::new(crate::system_eq_linux::LinuxSystemEq::start(state)?);
        #[cfg(target_os = "windows")]
        let session: Box<dyn Send> =
            Box::new(crate::system_eq_windows::WindowsSystemEq::start(state)?);
        *self.system_eq.lock().expect("system_eq poisoned") = Some(session);
        self.system_eq_status
            .store(SystemEqStatus::Active as u8, Ordering::Relaxed);
        Ok(())
    }

    /// Stop the self-contained system-wide EQ pipeline and restore audio routing
    /// (dropping the session runs its teardown). No-op if it isn't running.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub fn stop_system_eq(&self) {
        *self.system_eq.lock().expect("system_eq poisoned") = None;
        self.system_eq_status
            .store(SystemEqStatus::Disabled as u8, Ordering::Relaxed);
    }

    /// Stop playback.
    pub fn stop(&self) {
        let _ = self
            .ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::Stop);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        self.watchdog_alive.store(false, Ordering::Relaxed);
        if let Ok(tx) = self.ctrl.lock() {
            let _ = tx.send(EngineCommand::Shutdown);
        }
    }
}

/// Resolve the default output device and a usable f32 stream config (once).
fn output_setup() -> Result<(cpal::Device, cpal::StreamConfig), AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| AudioError::DeviceNotFound("no default output device".into()))?;
    let config = pick_f32_config(&device)?;
    Ok((device, config.config()))
}

fn pick_f32_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, AudioError> {
    let default = device.default_output_config().ok();
    if let Some(default) = default {
        if default.sample_format() == cpal::SampleFormat::F32 {
            return Ok(default);
        }
    }
    // Fall back to an f32 range — but never blindly at its *maximum* rate: DACs
    // advertising 192/384 kHz would make every DSP stage 4–8× as expensive.
    // Prefer the device's default rate (else 48 kHz), clamped into the range.
    let preferred = default.map(|d| d.sample_rate());
    let configs = device
        .supported_output_configs()
        .map_err(|e| AudioError::Host(e.to_string()))?;
    for range in configs {
        if range.sample_format() == cpal::SampleFormat::F32 {
            let rate = closest_supported_rate(
                range.min_sample_rate(),
                range.max_sample_rate(),
                preferred,
            );
            if let Some(config) = range.try_with_sample_rate(rate) {
                return Ok(config);
            }
        }
    }
    Err(AudioError::UnsupportedFormat(
        "no f32 output configuration available".into(),
    ))
}

/// The sample rate within `[min, max]` closest to `preferred` (the device's
/// default-config rate, when known), else closest to 48 kHz. Shared by the
/// output and input (capture) non-f32-default fallbacks so neither ever picks
/// a range's maximum outright. `min <= max` (as cpal guarantees for a range).
pub(crate) fn closest_supported_rate(min: u32, max: u32, preferred: Option<u32>) -> u32 {
    preferred.unwrap_or(48_000).clamp(min, max)
}

/// The engine control thread: owns the (`!Send`) cpal stream for its lifetime.
#[allow(clippy::too_many_arguments)]
/// Everything the engine control thread needs (grouped to keep the signature
/// readable and lock-free handles cloned once).
struct ControlCtx {
    rx: mpsc::Receiver<EngineCommand>,
    shared: Arc<ArcSwap<EngineState>>,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    track_meta: Arc<ArcSwap<TrackMeta>>,
    meta_version: Arc<AtomicU64>,
    queue_index: Arc<AtomicUsize>,
    crossfade: Arc<AtomicU32>,
    stem_gains: Arc<StemGains>,
    /// The chain's externally-owned slots, assembled once and cloned per stream.
    chain_slots: ChainSlots,
    output_beat: Arc<AtomicU64>,
    tap_active: Arc<AtomicBool>,
    /// Set by the watchdog when it requests a tap rebuild and cleared by the
    /// control thread once that rebuild attempt finishes, so only one rebuild is
    /// ever in flight (no create/destroy storm while the heartbeat is frozen).
    tap_rebuild_pending: Arc<AtomicBool>,
    /// User-facing system-EQ status; the control thread sets `Active` on a good
    /// (re)build, `Recovering` while rebuilding / cooling down, `Disabled` on stop.
    /// macOS-only in the control thread — the Linux/Windows self-contained EQ sets
    /// its status directly in `start_system_eq` / `stop_system_eq`.
    #[cfg(target_os = "macos")]
    system_eq_status: Arc<AtomicU8>,
    /// Shared macOS tap capture telemetry (heartbeat + first-callback layout),
    /// passed to each rebuilt `SystemTapSource` and watched by the watchdog.
    #[cfg(target_os = "macos")]
    capture_tel: Arc<crate::system_tap::CaptureTelemetry>,
    /// Core Audio id of the output device the current tap was built against, so the
    /// watchdog can tell a real default-device change from a mere output stall.
    #[cfg(target_os = "macos")]
    tap_output_device_id: Arc<AtomicU32>,
    /// Raised by the control thread when it gives up a rebuild burst (audio is
    /// restored unequalised); the watchdog reads it to start a cool-down before the
    /// next retry, so system EQ is never disabled permanently or silently.
    #[cfg(target_os = "macos")]
    tap_gave_up: Arc<AtomicBool>,
}

/// What restoring a state should do to the script slot.
///
/// Split out from the engine because the interesting part is the policy, and the
/// engine can only be built with a real audio device attached — untestable for
/// the sake of three branches that are entirely decidable from a string.
#[derive(Debug)]
enum SlotUpdate {
    /// No script: clear the slot so a previous one stops playing.
    Clear,
    /// Compiled: publish it.
    Publish(hm_dsp::script::Program),
    /// Did not compile: leave whatever is loaded alone.
    ///
    /// This is a restore or a preset apply, not someone typing. There is nobody
    /// to show the error to, and silencing a working chain over one stale field
    /// is worse than ignoring the field.
    Keep,
}

fn slot_update(source: &str) -> SlotUpdate {
    if source.trim().is_empty() {
        return SlotUpdate::Clear;
    }
    match hm_dsp::script::compile(source) {
        Ok(program) => SlotUpdate::Publish(program),
        Err(_) => SlotUpdate::Keep,
    }
}

/// Apply [`slot_update`]'s verdict to a slot.
///
/// Takes the slot rather than the engine so the outcome that matters most can
/// actually be asserted: that `Keep` leaves the previously-published program
/// *in place*. An engine cannot be built in a test without an audio device.
fn publish_script(slot: &ScriptSlot, source: &str) {
    match slot_update(source) {
        SlotUpdate::Clear => slot.store(std::sync::Arc::new(None)),
        SlotUpdate::Publish(program) => {
            slot.store(std::sync::Arc::new(Some(std::sync::Arc::new(program))))
        }
        SlotUpdate::Keep => {}
    }
}

fn control_loop(ctx: ControlCtx) {
    let ControlCtx {
        rx,
        shared,
        meters,
        spectrum,
        pos,
        playing,
        paused,
        track_meta,
        meta_version,
        queue_index,
        crossfade,
        stem_gains,
        chain_slots,
        output_beat,
        tap_active,
        tap_rebuild_pending,
        #[cfg(target_os = "macos")]
        system_eq_status,
        #[cfg(target_os = "macos")]
        capture_tel,
        #[cfg(target_os = "macos")]
        tap_output_device_id,
        #[cfg(target_os = "macos")]
        tap_gave_up,
    } = ctx;
    let setup = output_setup().ok();
    // macOS tap-recovery budget: after this many consecutive failed rebuilds we
    // stop the current burst, restore audio, and hand off to a watchdog cool-down
    // rather than churn coreaudiod.
    #[cfg(target_os = "macos")]
    let mut rebuild_policy = RebuildPolicy::new(4);
    // The active stream is held only to keep audio flowing (RAII); replacing or
    // taking it drops the previous one, stopping its callback.
    let mut active: Option<cpal::Stream> = None;
    // Set by the output callback once its source is fully exhausted (end of a
    // track/queue — live sources never signal it, and a paused stream's callback
    // doesn't run). Polled below so the device stream — whose callback would
    // otherwise keep running the full DSP chain + spectrum on silence — is torn
    // down shortly after EOF instead of only on the next command.
    let exhausted = Arc::new(AtomicBool::new(false));
    // Poll cadence for that post-EOF teardown while no command is arriving.
    const EOF_POLL: std::time::Duration = std::time::Duration::from_millis(250);

    loop {
        let cmd = match rx.recv_timeout(EOF_POLL) {
            Ok(cmd) => cmd,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Drop the stream the same way an explicit Stop does, but keep
                // position / now-playing metadata / paused untouched so the UI
                // (and OS media controls) still shows the finished track exactly
                // as it does today. Meters/spectrum are zeroed to match their
                // steady state after rendering silence. The `paused` guard keeps
                // a paused-at-EOF stream alive (pause ≠ exhausted); any later
                // Play command builds a fresh stream as usual.
                if exhausted.load(Ordering::Relaxed) && !paused.load(Ordering::Relaxed) {
                    exhausted.store(false, Ordering::Relaxed);
                    if active.take().is_some() {
                        meters.zero();
                        spectrum.zero();
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        // Disengage the macOS system-tap watchdog for any command that installs a
        // non-tap source or stops playback, so it never rebuilds a tap over
        // ordinary playback. The tap commands set/keep it; Pause/Resume/Shutdown
        // leave it untouched. (`tap_active` is engine-wide; on non-macOS it is
        // only ever cleared here and never read.)
        match &cmd {
            EngineCommand::PlaySource(_)
            | EngineCommand::Pause
            | EngineCommand::Resume
            | EngineCommand::Shutdown => {}
            #[cfg(target_os = "macos")]
            EngineCommand::RestartSystemTap => {}
            _ => {
                tap_active.store(false, Ordering::Relaxed);
                // Normal playback superseded the macOS tap: system EQ is off. (The
                // Linux/Windows self-contained EQ is independent of playback, so it
                // is left to `start_system_eq`/`stop_system_eq` to set its status.)
                #[cfg(target_os = "macos")]
                system_eq_status.store(SystemEqStatus::Disabled as u8, Ordering::Relaxed);
            }
        }
        match cmd {
            EngineCommand::Play(audio) => {
                drop(active.take()); // stop & release any current stream first
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                let resampled = resample_stereo(&audio.samples, audio.sample_rate, sample_rate);
                pos.prepare(sample_rate, resampled.len() / 2);
                let source = Box::new(FilePlaybackSource::new(resampled));
                // Publish the file's tags + cover for the now-playing UI.
                track_meta.store(Arc::new(audio.meta));
                meta_version.fetch_add(1, Ordering::Release);

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayStems { stems, start_secs } => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                // Resample each stem to the device rate (empty stems stay empty).
                let stems = *stems;
                let resampled: [Vec<f32>; STEM_COUNT] = stems
                    .map(|s| resample_stereo(&s.samples, s.sample_rate, sample_rate));
                let frames = resampled.iter().map(|s| s.len() / 2).max().unwrap_or(0);
                pos.prepare(sample_rate, frames);
                let mut stem_source =
                    StemPlaybackSource::new(resampled, stem_gains.clone(), sample_rate as f32);
                // Start at the live playhead so swapping stems in mid-track is gapless.
                let start_frame =
                    ((start_secs.max(0.0)) * sample_rate as f64).round() as usize;
                if start_frame > 0 {
                    stem_source.seek(start_frame);
                }
                let source = Box::new(stem_source);

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayStream {
                url,
                headers,
                duration_hint,
            } => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                pos.prepare(sample_rate, 0); // duration learned by the source (if any)
                // The stream thread publishes tags + cover once it has probed.
                let sink = MetaSink {
                    meta: track_meta.clone(),
                    version: meta_version.clone(),
                };
                let data_saver = shared.load().playback.data_saver;
                let tuning = StreamTuning::for_network(sample_rate, data_saver);
                let source = Box::new(RadioStreamSource::with_headers(
                    url,
                    headers,
                    sample_rate,
                    Some(sink),
                    duration_hint,
                    tuning,
                ));

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayQueue { paths, start } => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                pos.prepare(sample_rate, 0); // per-track totals reported by the source
                let sink = MetaSink {
                    meta: track_meta.clone(),
                    version: meta_version.clone(),
                };
                let source = Box::new(QueuePlaybackSource::spawn(
                    paths,
                    start,
                    sample_rate,
                    crossfade.clone(),
                    Some(sink),
                    Some(queue_index.clone()),
                ));

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayStreamQueue {
                resolver,
                count,
                start,
            } => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                pos.prepare(sample_rate, 0); // per-track totals reported by the source
                let sink = MetaSink {
                    meta: track_meta.clone(),
                    version: meta_version.clone(),
                };
                let source = Box::new(StreamQueueSource::spawn(
                    resolver,
                    count,
                    start,
                    sample_rate,
                    crossfade.clone(),
                    Some(sink),
                    Some(queue_index.clone()),
                ));

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayCapture => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                let Some((device, config)) = &setup else {
                    playing.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                pos.prepare(sample_rate, 0); // live capture: no duration
                match LoopbackCaptureSource::new(sample_rate) {
                    Ok(source) => match build_output_stream(
                        device,
                        *config,
                        Box::new(source),
                        shared.clone(),
                        meters.clone(),
                        spectrum.clone(),
                        pos.clone(),
                        playing.clone(),
                        channels,
                        sample_rate as f32,
                        chain_slots.clone(),
                        output_beat.clone(),
                        exhausted.clone(),
                    ) {
                        Ok(s) if s.play().is_ok() => {
                            playing.store(true, Ordering::Relaxed);
                            active = Some(s);
                        }
                        _ => playing.store(false, Ordering::Relaxed),
                    },
                    Err(_) => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlaySource(source) => {
                drop(active.take());
                meters.zero();
                spectrum.zero();
                paused.store(false, Ordering::Relaxed);
                // A fresh manual (re)enable supersedes any in-flight watchdog
                // rebuild / cool-down and starts the recovery budget over.
                tap_rebuild_pending.store(false, Ordering::Relaxed);
                #[cfg(target_os = "macos")]
                {
                    rebuild_policy.on_success();
                    tap_gave_up.store(false, Ordering::Relaxed);
                }
                let Some((device, config)) = &setup else {
                    crate::diag::log("PlaySource: NO OUTPUT SETUP — aborting");
                    playing.store(false, Ordering::Relaxed);
                    tap_active.store(false, Ordering::Relaxed);
                    #[cfg(target_os = "macos")]
                    system_eq_status.store(SystemEqStatus::Disabled as u8, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                crate::diag::log(&format!(
                    "PlaySource: output config channels={channels} sample_rate={sample_rate}"
                ));
                pos.prepare(sample_rate, 0); // live source: no duration

                match build_output_stream(
                    device,
                    *config,
                    source,
                    shared.clone(),
                    meters.clone(),
                    spectrum.clone(),
                    pos.clone(),
                    playing.clone(),
                    channels,
                    sample_rate as f32,
                    chain_slots.clone(),
                    output_beat.clone(),
                    exhausted.clone(),
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        // This is the macOS system tap: arm the watchdog so a dead
                        // output stream is rebuilt instead of muting the system.
                        tap_active.store(true, Ordering::Relaxed);
                        // Record the output device this tap was built against and
                        // mark system EQ Active, so the watchdog can later tell a
                        // real device change from a mere stall.
                        #[cfg(target_os = "macos")]
                        {
                            let dev = crate::system_tap::default_output_device_id().unwrap_or(0);
                            tap_output_device_id.store(dev, Ordering::Relaxed);
                            system_eq_status.store(SystemEqStatus::Active as u8, Ordering::Relaxed);
                        }
                        active = Some(s);
                    }
                    _ => {
                        playing.store(false, Ordering::Relaxed);
                        tap_active.store(false, Ordering::Relaxed);
                        #[cfg(target_os = "macos")]
                        system_eq_status.store(SystemEqStatus::Disabled as u8, Ordering::Relaxed);
                    }
                }
            }
            // Watchdog-driven recovery: the tap's output/capture stream died (e.g. a
            // default-output-device change, or a dead capture io_proc). Rebuild a
            // fresh tap on the *current* default device. Re-resolving here (rather
            // than the frozen startup `setup`) is what lets the tap follow the new
            // device. The watchdog has already confirmed (via `assess_tap_stall`)
            // that this is a real death/change and not mere CPU starvation.
            //
            // The watchdog sets `tap_rebuild_pending` before sending this and we
            // clear it on every exit, so at most one rebuild is ever in flight —
            // no create/destroy storm (which used to wedge coreaudiod and strand a
            // muting tap). When `rebuild_policy` exhausts its per-burst budget we do
            // NOT disable system EQ forever: we restore audio (unequalised) and set
            // `tap_gave_up`, handing off to the watchdog's exponential cool-down
            // re-arm — so the tap keeps trying to come back and the UI can observe
            // `Recovering` instead of a silent permanent failure.
            #[cfg(target_os = "macos")]
            EngineCommand::RestartSystemTap => {
                if !tap_active.load(Ordering::Relaxed) {
                    tap_rebuild_pending.store(false, Ordering::Relaxed);
                    continue; // user turned the tap off in the meantime
                }
                system_eq_status.store(SystemEqStatus::Recovering as u8, Ordering::Relaxed);
                crate::diag::log("RestartSystemTap: rebuilding tap on current default device");
                drop(active.take()); // tear down old tap (unmutes) + dead stream
                let Ok((device, config)) = output_setup() else {
                    crate::diag::log("RestartSystemTap: no output device — will retry");
                    playing.store(false, Ordering::Relaxed);
                    if rebuild_policy.on_failure() {
                        tap_gave_up.store(true, Ordering::Relaxed);
                        crate::diag::log(
                            "RestartSystemTap: no output device after repeated tries — audio \
                             restored; watchdog will retry after a cool-down",
                        );
                    }
                    tap_rebuild_pending.store(false, Ordering::Relaxed);
                    continue;
                };
                let sample_rate = config.sample_rate;
                let channels = config.channels as usize;
                pos.prepare(sample_rate, 0);
                // Read the scope *now*, not at the time the tap was first built:
                // a rebuild has to honour the selection as it currently stands,
                // or a watchdog recovery would quietly widen a user's per-app
                // choice back to the whole system.
                let scope = shared.load().system_eq_scope.clone();
                let rebuilt = match SystemTapSource::new(sample_rate, capture_tel.clone(), &scope) {
                    Ok(source) => match build_output_stream(
                        &device,
                        config,
                        Box::new(source),
                        shared.clone(),
                        meters.clone(),
                        spectrum.clone(),
                        pos.clone(),
                        playing.clone(),
                        channels,
                        sample_rate as f32,
                        chain_slots.clone(),
                        output_beat.clone(),
                        exhausted.clone(),
                    ) {
                        Ok(s) if s.play().is_ok() => {
                            playing.store(true, Ordering::Relaxed);
                            // Bump the heartbeat so the watchdog sees progress and
                            // doesn't immediately re-fire before the first callback.
                            output_beat.fetch_add(1, Ordering::Relaxed);
                            // Record the (possibly new) output device this tap runs on.
                            let dev =
                                crate::system_tap::default_output_device_id().unwrap_or(0);
                            tap_output_device_id.store(dev, Ordering::Relaxed);
                            active = Some(s);
                            crate::diag::log("RestartSystemTap: tap rebuilt OK");
                            true
                        }
                        _ => {
                            playing.store(false, Ordering::Relaxed);
                            crate::diag::log("RestartSystemTap: stream build failed — will retry");
                            false
                        }
                    },
                    Err(e) => {
                        playing.store(false, Ordering::Relaxed);
                        crate::diag::log(&format!(
                            "RestartSystemTap: tap creation failed ({e:?}) — will retry"
                        ));
                        false
                    }
                };
                if rebuilt {
                    rebuild_policy.on_success();
                    system_eq_status.store(SystemEqStatus::Active as u8, Ordering::Relaxed);
                } else if rebuild_policy.on_failure() {
                    // Per-burst budget exhausted: stop hammering coreaudiod for now.
                    // Restore audio (drop any muting tap — `active` is already None
                    // on the failure paths, drop again to be certain) and hand off
                    // to the watchdog's cool-down re-arm. tap_active stays TRUE (the
                    // user still wants system EQ); status is Recovering, not off.
                    drop(active.take());
                    tap_gave_up.store(true, Ordering::Relaxed);
                    system_eq_status.store(SystemEqStatus::Recovering as u8, Ordering::Relaxed);
                    crate::diag::log(
                        "RestartSystemTap: backing off after repeated failures — audio \
                         restored (unequalised); watchdog will retry after a cool-down",
                    );
                }
                // else: within the per-burst budget — leave status Recovering; the
                // watchdog's stall detection will pace the next attempt.
                tap_rebuild_pending.store(false, Ordering::Relaxed);
            }
            EngineCommand::Pause => {
                if let Some(s) = &active {
                    if s.pause().is_ok() {
                        paused.store(true, Ordering::Relaxed);
                    }
                }
            }
            EngineCommand::Resume => {
                if let Some(s) = &active {
                    if s.play().is_ok() {
                        paused.store(false, Ordering::Relaxed);
                    }
                }
            }
            EngineCommand::Stop => {
                drop(active.take());
                playing.store(false, Ordering::Relaxed);
                paused.store(false, Ordering::Relaxed);
                meters.zero();
                spectrum.zero();
                pos.reset();
                // Stopping playback also stops the macOS tap (it plays through the
                // same output stream). `tap_active` was cleared by the pre-match
                // guard; reflect the same in the status.
                #[cfg(target_os = "macos")]
                system_eq_status.store(SystemEqStatus::Disabled as u8, Ordering::Relaxed);
            }
            EngineCommand::Shutdown => break,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_output_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    source: Box<dyn AudioSource>,
    shared: Arc<ArcSwap<EngineState>>,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    channels: usize,
    sample_rate: f32,
    slots: ChainSlots,
    output_beat: Arc<AtomicU64>,
    exhausted: Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError> {
    let mut renderer = Renderer::new(source, sample_rate, channels, slots);
    // A fresh stream starts un-exhausted: the flag may be stale from a previous
    // source, and the control loop must never tear *this* stream down for it.
    exhausted.store(false, Ordering::Relaxed);

    device
        .build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Flush denormals on whichever thread runs this callback (it can
                // migrate on some backends): decaying IIR tails otherwise reach
                // denormal range, at 50–100+ cycles per multiply on x86.
                crate::thread_util::enable_denormal_flush_once();
                // Liveness heartbeat: a frozen counter is how the macOS tap
                // watchdog detects this stream has died. One relaxed add, RT-safe.
                output_beat.fetch_add(1, Ordering::Relaxed);
                let state = shared.load_full();
                let at_end =
                    renderer.render(data, channels, state.as_ref(), &meters, &spectrum, &pos);
                if at_end {
                    playing.store(false, Ordering::Relaxed);
                    // Ask the control thread to tear this stream down (post-EOF
                    // it would only be rendering the DSP chain over silence).
                    exhausted.store(true, Ordering::Relaxed);
                }
            },
            move |_err| {
                // Stream errors (e.g. device unplugged) end playback. For the
                // system tap the watchdog rebuilds it; otherwise the next play
                // command rebuilds. Nothing to do on the RT side.
            },
            None,
        )
        .map_err(|e| AudioError::Stream(e.to_string()))
}

/// Everything the macOS tap watchdog observes/controls. Bundled so the loop's
/// signature stays readable (and clippy-clean) as the recovery logic grew.
#[cfg(target_os = "macos")]
struct TapWatch {
    /// Cleared on engine drop so the loop exits.
    alive: Arc<AtomicBool>,
    /// Output-callback heartbeat (frozen ⇒ the output stream stalled/died).
    output_beat: Arc<AtomicU64>,
    /// Capture io_proc telemetry, incl. its heartbeat (frozen ⇒ capture stalled).
    capture_tel: Arc<crate::system_tap::CaptureTelemetry>,
    tap_active: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    /// One-rebuild-in-flight coalescing gate (shared with the control thread).
    rebuild_pending: Arc<AtomicBool>,
    /// Output device the current tap was built against (for change detection).
    tap_output_device_id: Arc<AtomicU32>,
    /// Raised by the control thread when a rebuild burst gave up — starts a cool-down.
    tap_gave_up: Arc<AtomicBool>,
    /// Raised by the default-output-device Core Audio listener (allocation-free).
    device_change_signal: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    ctrl: mpsc::Sender<EngineCommand>,
}

/// macOS tap watchdog loop. Samples the output *and* capture heartbeats every
/// `TICK` and, while a tap session is engaged and not paused, decides via
/// [`assess_tap_stall`] whether a frozen heartbeat is a real device death/change
/// (rebuild) or mere CPU starvation on a live device (back off, keep the tap).
/// This is what stops a heavy-load stall from needlessly tearing down the tap —
/// the "suddenly stops under load" bug — while still recovering from genuine
/// failures, including a dead *capture* proc that the old output-only heartbeat
/// couldn't see (which left the system muted with a healthy output heartbeat).
///
/// Recovery is never permanent-off: after the control thread exhausts a rebuild
/// burst it raises `tap_gave_up`, and this loop schedules an exponential cool-down
/// ([`cooldown_secs`]) before re-arming another attempt — so system EQ keeps
/// trying to come back and the status reflects `Recovering`, never a silent death.
///
/// `rebuild_pending` gates re-firing: once a rebuild is requested it stays set
/// until the control thread finishes that attempt, so the watchdog never queues a
/// second rebuild while one is in progress (the create/destroy storm that wedged
/// coreaudiod). The device-change listener coalesces through the same gate.
#[cfg(target_os = "macos")]
fn tap_watchdog(watch: TapWatch) {
    use std::time::{Duration, Instant};
    /// Sampling cadence. Short enough to recover quickly, long enough to be free.
    const TICK: Duration = Duration::from_millis(250);
    /// Consecutive frozen samples before declaring a heartbeat stalled (~750 ms).
    const STALL_TICKS: u32 = 3;
    /// How many times a live-device stall may back off (waiting out suspected CPU
    /// starvation) before we attempt a bounded rebuild anyway. `4 × ~750 ms ≈ 3 s`
    /// of patience — enough to ride out load spikes, bounded so a genuinely dead
    /// output on an "alive" device (e.g. same-device sample-rate change) still
    /// recovers.
    const MAX_BACKOFF: u32 = 4;

    let TapWatch {
        alive,
        output_beat,
        capture_tel,
        tap_active,
        paused,
        rebuild_pending,
        tap_output_device_id,
        tap_gave_up,
        device_change_signal,
        status,
        ctrl,
    } = watch;

    let mut out_detector = StallDetector::new();
    let mut cap_detector = StallDetector::new();
    let mut backoff_count: u32 = 0;
    // Consecutive give-ups and the current post-give-up cool-down deadline.
    let mut giveup_count: u32 = 0;
    let mut cooldown_until: Option<Instant> = None;
    // Tap generation last logged, so first-callback telemetry is logged once/tap.
    let mut logged_generation: u64 = 0;

    // Send a rebuild request iff we win the single in-flight slot. Returns false
    // when the control thread has gone (engine shutting down) so the loop can exit.
    let request_rebuild = |reason: &str| -> bool {
        if rebuild_pending.swap(true, Ordering::Relaxed) {
            return true; // a rebuild is already in flight — coalesce
        }
        crate::diag::log(reason);
        ctrl.send(EngineCommand::RestartSystemTap).is_ok()
    };

    while alive.load(Ordering::Relaxed) {
        std::thread::sleep(TICK);
        if !alive.load(Ordering::Relaxed) {
            break;
        }

        // A healthy/recovered tap (status Active) clears the give-up escalation and
        // any pending cool-down. NOTE: `backoff_count` is deliberately NOT reset
        // here — an ongoing output stall keeps the tap "active" (status stays
        // Active because we haven't torn it down), so resetting it here would let a
        // genuinely dead-but-"alive" device back off forever. It is cleared only by
        // real heartbeat progress (below) or when we act on a stall.
        if SystemEqStatus::from_u8(status.load(Ordering::Relaxed)) == SystemEqStatus::Active {
            giveup_count = 0;
            cooldown_until = None;
        }

        // One-shot, non-RT: log the first-callback buffer layout once per tap.
        let generation = capture_tel.generation();
        if generation != logged_generation {
            if let Some((buffers, channels, bytes, peak)) = capture_tel.first_layout() {
                crate::diag::log(&format!(
                    "tap io_proc first callback: n_buffers={buffers} channels={channels} \
                     bytes={bytes} peak={peak:.4}"
                ));
                logged_generation = generation;
            }
        }

        // 1. The control thread gave up a rebuild burst → start (or lengthen) the
        //    exponential cool-down before we re-arm. Audio is already restored.
        if tap_gave_up.swap(false, Ordering::Relaxed) {
            giveup_count = giveup_count.saturating_add(1);
            let secs = cooldown_secs(giveup_count);
            cooldown_until = Some(Instant::now() + Duration::from_secs(secs));
            out_detector.reset();
            cap_detector.reset();
            crate::diag::log(&format!(
                "tap watchdog: system EQ gave up (attempt {giveup_count}) — cooling down {secs}s \
                 before retry"
            ));
            continue;
        }

        // 2. In a post-give-up cool-down: hold off entirely (keep detectors primed).
        if let Some(deadline) = cooldown_until {
            if Instant::now() < deadline {
                out_detector.observe(output_beat.load(Ordering::Relaxed), false);
                cap_detector.observe(capture_tel.beat(), false);
                continue;
            }
            cooldown_until = None;
            // Cool-down elapsed: attempt exactly one rebuild (if still wanted).
            let active = tap_active.load(Ordering::Relaxed) && !paused.load(Ordering::Relaxed);
            if active && !request_rebuild("tap watchdog: cool-down elapsed — retrying system EQ") {
                break;
            }
            continue;
        }

        // 3. A rebuild is already in flight: don't sample toward another. Keep the
        //    detectors primed so a fresh window is required once it completes.
        if rebuild_pending.load(Ordering::Relaxed) {
            out_detector.observe(output_beat.load(Ordering::Relaxed), false);
            cap_detector.observe(capture_tel.beat(), false);
            continue;
        }

        let active = tap_active.load(Ordering::Relaxed) && !paused.load(Ordering::Relaxed);

        // 4. Proactive: the default output device changed (Core Audio listener).
        //    Rebuild immediately — a device change always needs a fresh tap.
        //    Always consume the signal; only act on it while the tap is engaged.
        let device_changed_now = device_change_signal.swap(false, Ordering::Relaxed);
        if device_changed_now && active {
            out_detector.reset();
            cap_detector.reset();
            backoff_count = 0;
            if !request_rebuild("tap watchdog: default output device changed — rebuilding tap") {
                break;
            }
            continue;
        }

        // 5. Heartbeat stall assessment.
        let out_misses = out_detector.observe(output_beat.load(Ordering::Relaxed), active);
        let cap_misses = cap_detector.observe(capture_tel.beat(), active);
        let output_stalled = out_misses >= STALL_TICKS;
        let capture_stalled = cap_misses >= STALL_TICKS;

        // Both heartbeats progressing while active ⇒ genuinely healthy; forget any
        // accumulated starvation patience so an unrelated later stall gets it fresh.
        if active && out_misses == 0 && cap_misses == 0 {
            backoff_count = 0;
        }

        if active && (output_stalled || capture_stalled) {
            let built = tap_output_device_id.load(Ordering::Relaxed);
            let current = crate::system_tap::default_output_device_id();
            let device_changed = matches!(current, Some(c) if built != 0 && c != built);
            let device_alive = crate::system_tap::device_is_alive(built);
            let backoff_exhausted = backoff_count >= MAX_BACKOFF;

            match assess_tap_stall(
                output_stalled,
                capture_stalled,
                device_alive,
                device_changed,
                backoff_exhausted,
            ) {
                TapAction::Rebuild => {
                    out_detector.reset();
                    cap_detector.reset();
                    backoff_count = 0;
                    if !request_rebuild(
                        "tap watchdog: confirmed stall (device dead/changed or dead capture) \
                         — rebuilding tap",
                    ) {
                        break;
                    }
                }
                TapAction::BackOff => {
                    // Live, unchanged device — treat as CPU starvation: keep the
                    // tap, extend the wait. Don't churn coreaudiod under load.
                    out_detector.reset();
                    cap_detector.reset();
                    backoff_count = backoff_count.saturating_add(1);
                    crate::diag::log(
                        "tap watchdog: heartbeat frozen but device alive & unchanged — likely \
                         CPU load; keeping tap and backing off",
                    );
                }
                TapAction::Healthy => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::FilePlaybackSource;

    /// A stored script is only *text*; the compiled program it needs cannot be
    /// serialized. So every path that replaces state wholesale — launch restore,
    /// preset apply — has to rebuild it, or the UI shows an enabled script with
    /// the user's code in it over a chain running identity.
    #[test]
    fn a_restored_script_is_compiled_back_into_the_slot() {
        let update = slot_update("@sample\n  spl0 = spl0 * 0.5;\n");
        assert!(
            matches!(update, SlotUpdate::Publish(_)),
            "a compiling source must be published, got {update:?}"
        );
    }

    /// Nothing to run: the slot must be cleared, not left holding whatever the
    /// previous state's script was.
    #[test]
    fn an_empty_source_clears_the_slot() {
        assert!(matches!(slot_update(""), SlotUpdate::Clear));
        assert!(matches!(slot_update("   \n\t "), SlotUpdate::Clear));
    }

    /// A preset carrying a script that no longer compiles must not silence the
    /// chain around it. There is no one to report the error to mid-restore, and
    /// one bad field should not reject the other twenty.
    #[test]
    fn a_source_that_will_not_compile_leaves_the_running_program_alone() {
        assert!(matches!(slot_update("@sample this is not eel2 ((("), SlotUpdate::Keep));
    }

    /// The same claim, asked of a real slot — `Keep` is the one verdict whose
    /// whole meaning is what it *doesn't* do, and a match arm that fell through
    /// to a clear would satisfy every assertion above.
    #[test]
    fn a_broken_source_does_not_disturb_what_is_already_playing() {
        let slot = hm_dsp::empty_script_slot();
        publish_script(&slot, "@sample\n  spl0 = spl0 * 0.5;\n");
        assert!(slot.load_full().is_some(), "a compiling source must publish");

        publish_script(&slot, "@sample not ((( eel2");
        assert!(
            slot.load_full().is_some(),
            "a broken source must leave the running program in place, not clear it"
        );

        // An empty source *is* a reason to clear: there is nothing to run.
        publish_script(&slot, "   ");
        assert!(slot.load_full().is_none(), "an empty source must clear the slot");
    }

    /// Compilation is keyed on the source, never on `enabled` — so switching the
    /// toggle on after a restart takes effect immediately rather than silently
    /// requiring a trip through Apply first.
    #[test]
    fn a_disabled_script_is_still_compiled() {
        let mut state = EngineState::default();
        state.script.enabled = false;
        state.script.source = "@sample\n  spl0 = spl0 * 0.5;\n".into();
        assert!(matches!(slot_update(&state.script.source), SlotUpdate::Publish(_)));
    }

    const THRESH: u32 = 3;

    #[test]
    fn stall_detector_stays_zero_when_inactive() {
        let mut d = StallDetector::new();
        // A frozen heartbeat is irrelevant while no tap session is live (or paused).
        for _ in 0..(THRESH * 3) {
            assert_eq!(d.observe(42, false), 0);
        }
    }

    #[test]
    fn stall_detector_stays_zero_while_output_progresses() {
        let mut d = StallDetector::new();
        for beat in 1..=(THRESH * 3) as u64 {
            assert_eq!(d.observe(beat, true), 0, "advancing output is healthy");
        }
    }

    #[test]
    fn stall_detector_counts_consecutive_frozen_beats() {
        let mut d = StallDetector::new();
        // Prime with a healthy beat, then freeze: the count climbs each tick.
        assert_eq!(d.observe(100, true), 0);
        assert_eq!(d.observe(100, true), 1);
        assert_eq!(d.observe(100, true), 2);
        assert_eq!(d.observe(100, true), 3, "reaches the stall threshold");
        assert_eq!(d.observe(100, true), 4, "keeps climbing while frozen");
    }

    #[test]
    fn stall_detector_reset_rearms_the_window() {
        let mut d = StallDetector::new();
        assert_eq!(d.observe(7, true), 0);
        assert_eq!(d.observe(7, true), 1);
        assert_eq!(d.observe(7, true), 2);
        // The watchdog acts on a stall then re-arms: a fresh window must start over.
        d.reset();
        assert_eq!(d.observe(7, true), 1, "counts from zero after reset");
    }

    #[test]
    fn stall_detector_resets_on_recovery() {
        let mut d = StallDetector::new();
        assert_eq!(d.observe(5, true), 0);
        assert_eq!(d.observe(5, true), 1);
        // Output recovers before the threshold — counter must reset.
        assert_eq!(d.observe(6, true), 0);
        // A fresh freeze needs the full window again.
        assert_eq!(d.observe(6, true), 1);
        assert_eq!(d.observe(6, true), 2);
        assert_eq!(d.observe(6, true), 3);
    }

    #[test]
    fn stall_detector_pause_resets_miss_count() {
        let mut d = StallDetector::new();
        assert_eq!(d.observe(9, true), 0);
        assert_eq!(d.observe(9, true), 1);
        assert_eq!(d.observe(9, true), 2);
        // Pausing (active=false) clears the count even with a frozen beat.
        assert_eq!(d.observe(9, false), 0);
        // Resuming: a frozen beat must again take the full window to reach THRESH.
        assert_eq!(d.observe(9, true), 1);
        assert_eq!(d.observe(9, true), 2);
        assert_eq!(d.observe(9, true), 3);
    }

    // --- assess_tap_stall: the starvation-vs-death decision -------------------

    #[test]
    fn assess_healthy_when_nothing_stalled() {
        // No stall on either heartbeat ⇒ nothing to do, regardless of device state.
        assert_eq!(
            assess_tap_stall(false, false, true, false, false),
            TapAction::Healthy
        );
        assert_eq!(
            assess_tap_stall(false, false, false, true, true),
            TapAction::Healthy
        );
    }

    #[test]
    fn assess_output_stall_on_live_unchanged_device_backs_off() {
        // Chain A: output frozen but the device is alive and unchanged — this is
        // CPU starvation, NOT death. Must NOT tear the tap down.
        assert_eq!(
            assess_tap_stall(true, false, true, false, false),
            TapAction::BackOff
        );
        // Both frozen on a live, unchanged device is still treated as starvation.
        assert_eq!(
            assess_tap_stall(true, true, true, false, false),
            TapAction::BackOff
        );
    }

    #[test]
    fn assess_rebuilds_on_device_death_or_change() {
        // A dead device → rebuild even though we'd otherwise back off.
        assert_eq!(
            assess_tap_stall(true, false, false, false, false),
            TapAction::Rebuild
        );
        // A default-device change → rebuild on the new device.
        assert_eq!(
            assess_tap_stall(true, false, true, true, false),
            TapAction::Rebuild
        );
    }

    #[test]
    fn assess_rebuilds_on_dead_capture_while_output_runs() {
        // Chain B: capture frozen while output still ticks proves callbacks run,
        // so the capture io_proc is genuinely dead — only a rebuild restarts it.
        assert_eq!(
            assess_tap_stall(false, true, true, false, false),
            TapAction::Rebuild
        );
    }

    #[test]
    fn assess_rebuilds_after_backoff_is_exhausted() {
        // A live-device output stall we've already waited out (e.g. a same-device
        // sample-rate change) must eventually rebuild rather than wait forever.
        assert_eq!(
            assess_tap_stall(true, false, true, false, true),
            TapAction::Rebuild
        );
    }

    // --- cooldown_secs: exponential back-off, capped --------------------------

    #[test]
    fn cooldown_grows_exponentially_then_caps() {
        assert_eq!(cooldown_secs(1), 30);
        assert_eq!(cooldown_secs(2), 60);
        assert_eq!(cooldown_secs(3), 120);
        assert_eq!(cooldown_secs(4), 240);
        assert_eq!(cooldown_secs(5), 300, "capped at 300s");
        assert_eq!(cooldown_secs(9), 300, "stays capped and never overflows");
        // Defensive: a 0th give-up (shouldn't happen) still yields a sane value.
        assert_eq!(cooldown_secs(0), 30);
    }

    #[test]
    fn system_eq_status_u8_roundtrips() {
        for s in [
            SystemEqStatus::Disabled,
            SystemEqStatus::Active,
            SystemEqStatus::Recovering,
        ] {
            assert_eq!(SystemEqStatus::from_u8(s as u8), s);
        }
        // Unknown byte values fall back to Disabled (safe default).
        assert_eq!(SystemEqStatus::from_u8(200), SystemEqStatus::Disabled);
    }

    #[test]
    fn rebuild_policy_gives_up_after_max_consecutive_failures() {
        let mut p = RebuildPolicy::new(4);
        assert!(!p.on_failure(), "1st failure within budget");
        assert!(!p.on_failure(), "2nd failure within budget");
        assert!(!p.on_failure(), "3rd failure within budget");
        assert!(p.on_failure(), "4th consecutive failure exhausts the budget");
    }

    #[test]
    fn rebuild_policy_success_resets_the_budget() {
        let mut p = RebuildPolicy::new(3);
        assert!(!p.on_failure());
        assert!(!p.on_failure());
        p.on_success(); // a recovered device must not count toward give-up
        assert!(!p.on_failure());
        assert!(!p.on_failure());
        assert!(p.on_failure(), "give up only on 3 *consecutive* failures");
    }

    #[test]
    fn rebuild_policy_rearms_after_giving_up() {
        let mut p = RebuildPolicy::new(2);
        assert!(!p.on_failure());
        assert!(p.on_failure(), "gives up");
        // A later session (after the tap is re-enabled) starts with a fresh budget.
        assert!(!p.on_failure());
        assert!(p.on_failure(), "fires again after re-arming");
    }

    #[test]
    fn closest_supported_rate_prefers_the_device_default() {
        // Device default inside the range: use it verbatim.
        assert_eq!(closest_supported_rate(8_000, 192_000, Some(44_100)), 44_100);
        // Device default outside the range: clamp to the nearest edge.
        assert_eq!(closest_supported_rate(48_000, 192_000, Some(44_100)), 48_000);
        assert_eq!(closest_supported_rate(8_000, 48_000, Some(96_000)), 48_000);
    }

    #[test]
    fn closest_supported_rate_falls_back_to_48k_never_the_max() {
        // No default known: aim for 48 kHz, clamped into the range — a
        // 192/384 kHz-capable DAC must not quadruple the whole DSP chain.
        assert_eq!(closest_supported_rate(8_000, 384_000, None), 48_000);
        assert_eq!(closest_supported_rate(8_000, 44_100, None), 44_100);
        assert_eq!(closest_supported_rate(96_000, 192_000, None), 96_000);
    }

    fn constant_source(amplitude: f32, frames: usize) -> Box<dyn AudioSource> {
        let mut s = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            s.push(amplitude);
            s.push(amplitude);
        }
        Box::new(FilePlaybackSource::new(s))
    }

    #[test]
    fn limiter_keeps_engaged_output_below_ceiling() {
        let mut renderer = Renderer::new(
            constant_source(2.0, 4096),
            48_000.0,
            2,
            ChainSlots::default(),
        );
        let meters = EngineMeters::default();
        let spectrum = SpectrumTap::default();
        let pos = PlaybackPos::default();
        let state = EngineState::default(); // power on, limiter on, ceiling -0.3 dB
        let ceiling = 10f32.powf(-0.3 / 20.0);

        let mut out = vec![0.0f32; 4096 * 2];
        renderer.render(&mut out, 2, &state, &meters, &spectrum, &pos);

        let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(
            peak <= ceiling + 1e-3,
            "engaged peak {peak} exceeded ceiling"
        );
        // Meters reflect real output (non-silent).
        assert!(meters.load().peak[0] > 0.0);
    }

    #[test]
    fn power_off_bypasses_the_chain() {
        let mut renderer = Renderer::new(
            constant_source(2.0, 2048),
            48_000.0,
            2,
            ChainSlots::default(),
        );
        let meters = EngineMeters::default();
        let spectrum = SpectrumTap::default();
        let pos = PlaybackPos::default();
        // Power off: chain bypassed, no limiting.
        let state = EngineState {
            power: false,
            ..Default::default()
        };

        let mut out = vec![0.0f32; 2048 * 2];
        renderer.render(&mut out, 2, &state, &meters, &spectrum, &pos);

        let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(
            peak > 1.5,
            "bypassed signal should pass at full level, got {peak}"
        );
    }

    #[test]
    fn meters_are_zero_for_silence() {
        let frame = compute_meters(&[0.0; 256], 2);
        assert_eq!(frame.peak, [0.0, 0.0]);
        assert_eq!(frame.rms, [0.0, 0.0]);
    }
}
