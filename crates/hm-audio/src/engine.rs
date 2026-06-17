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
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hm_core::{
    BassBoostState, EngineState, HeadphoneCorrectionState, MeterFrame, ParametricBand, SpatialMode,
    SpatializerState,
};
use hm_dsp::ProcessChain;

use crate::capture::LoopbackCaptureSource;
use crate::decode::{decode_file, resample_stereo, DecodedAudio};
use crate::error::AudioError;
use crate::sources::FilePlaybackSource;
use crate::spectrum::{Analyzer, SpectrumTap};
use crate::streaming::RadioStreamSource;
use crate::AudioSource;

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

/// Peak and RMS per channel over a processed block (first two channels).
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
}

impl Default for PlaybackPos {
    fn default() -> Self {
        Self {
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(0),
            sample_rate: AtomicU32::new(0),
            seek_to: AtomicI64::new(-1),
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
    pub fn new(source: Box<dyn AudioSource>, sample_rate: f32, channels: usize) -> Self {
        Self {
            chain: ProcessChain::standard(sample_rate, channels),
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
    PlayRadio(String),
    PlayCapture,
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
    ctrl: Mutex<mpsc::Sender<EngineCommand>>,
    _thread: JoinHandle<()>,
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
        let (tx, rx) = mpsc::channel();

        let thread = {
            let shared = shared.clone();
            let meters = meters.clone();
            let spectrum = spectrum.clone();
            let pos = pos.clone();
            let playing = playing.clone();
            let paused = paused.clone();
            std::thread::Builder::new()
                .name("hm-audio-engine".into())
                .spawn(move || control_loop(rx, shared, meters, spectrum, pos, playing, paused))
                .expect("failed to spawn hm-audio engine thread")
        };

        Self {
            write_state: Mutex::new(initial),
            shared,
            meters,
            spectrum,
            pos,
            playing,
            paused,
            ctrl: Mutex::new(tx),
            _thread: thread,
        }
    }

    /// Current engine state.
    pub fn state(&self) -> EngineState {
        self.write_state
            .lock()
            .expect("engine state poisoned")
            .clone()
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

    /// Replace the full engine state (used by EQ/preset application later).
    pub fn set_state(&self, new_state: EngineState) {
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
    pub fn set_bass(&self, enabled: bool, amount: f32, harmonics: bool) {
        self.update(|s| {
            s.bass = BassBoostState {
                enabled,
                amount,
                harmonics,
            };
        });
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

    /// Stream and play an internet radio URL through the chain.
    pub fn play_radio(&self, url: String) -> Result<(), AudioError> {
        self.ctrl
            .lock()
            .expect("engine ctrl poisoned")
            .send(EngineCommand::PlayRadio(url))
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
    if let Ok(default) = device.default_output_config() {
        if default.sample_format() == cpal::SampleFormat::F32 {
            return Ok(default);
        }
    }
    let configs = device
        .supported_output_configs()
        .map_err(|e| AudioError::Host(e.to_string()))?;
    for range in configs {
        if range.sample_format() == cpal::SampleFormat::F32 {
            return Ok(range.with_max_sample_rate());
        }
    }
    Err(AudioError::UnsupportedFormat(
        "no f32 output configuration available".into(),
    ))
}

/// The engine control thread: owns the (`!Send`) cpal stream for its lifetime.
#[allow(clippy::too_many_arguments)]
fn control_loop(
    rx: mpsc::Receiver<EngineCommand>,
    shared: Arc<ArcSwap<EngineState>>,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) {
    let setup = output_setup().ok();
    // The active stream is held only to keep audio flowing (RAII); replacing or
    // taking it drops the previous one, stopping its callback.
    let mut active: Option<cpal::Stream> = None;

    while let Ok(cmd) = rx.recv() {
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
                ) {
                    Ok(s) if s.play().is_ok() => {
                        playing.store(true, Ordering::Relaxed);
                        active = Some(s);
                    }
                    _ => playing.store(false, Ordering::Relaxed),
                }
            }
            EngineCommand::PlayRadio(url) => {
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
                pos.prepare(sample_rate, 0); // live stream: no known duration
                let source = Box::new(RadioStreamSource::new(url, sample_rate));

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
) -> Result<cpal::Stream, AudioError> {
    let mut renderer = Renderer::new(source, sample_rate, channels);

    device
        .build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let state = shared.load_full();
                let exhausted =
                    renderer.render(data, channels, state.as_ref(), &meters, &spectrum, &pos);
                if exhausted {
                    playing.store(false, Ordering::Relaxed);
                }
            },
            move |_err| {
                // Stream errors (e.g. device unplugged) end playback; the next
                // command rebuilds the stream. Nothing to do on the RT side.
            },
            None,
        )
        .map_err(|e| AudioError::Stream(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::FilePlaybackSource;

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
        let mut renderer = Renderer::new(constant_source(2.0, 4096), 48_000.0, 2);
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
        let mut renderer = Renderer::new(constant_source(2.0, 2048), 48_000.0, 2);
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
