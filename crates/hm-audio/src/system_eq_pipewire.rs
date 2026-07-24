//! System-wide EQ on **PipeWire** via a native client (the EasyEffects model).
//!
//! This is the macOS-parity Linux path: transparent, zero-config, crash-safe,
//! and low-latency. It exists because the classic virtual-sink + `pactl` trick
//! (see [`crate::system_eq_pulse`]) is unreliable on PipeWire — WirePlumber owns
//! routing policy, so `set-default-sink`/`move-sink-input` are advisory and apps
//! frequently keep playing to the real device *unprocessed*. EasyEffects solves
//! this by being a resident native client, and so do we.
//!
//! ## How it works
//!
//! Two PipeWire streams live on one worker-owned main loop:
//!
//! - a **sink stream** (`media.class = Audio/Sink`, `Direction::Input`) that
//!   *is* a selectable virtual sink named `hypemuzik_eq`. Every app routed to it
//!   is mixed by the graph and delivered to our process callback. That callback
//!   only pushes the mixed samples into a lock-free [`rtrb`] ring.
//! - an **output stream** (`Direction::Output`, `AUTOCONNECT`) that connects to
//!   the real default device. Its process callback pops the ring, runs the
//!   shared DSP [`process_block`], and writes the result to the device.
//!
//! Apps are routed to our sink not by changing the default (WirePlumber fights
//! that) but by the **stream mover**: a registry listener that, for every
//! `Stream/Output/Audio` node — except our own output and HypeMuzik's own
//! playback — sets `target.object` metadata to our sink. `target.object` *is*
//! WirePlumber's own routing mechanism, so it cooperates. New, existing, and
//! even user-pinned streams all get captured, and re-captured on every new-node
//! event.
//!
//! ## Crash-safety
//!
//! Our sink and streams are owned by our PipeWire client connection. If
//! HypeMuzik dies, the server destroys them automatically and WirePlumber
//! re-routes the freed app streams back to the real device — no system-wide
//! silence, unlike the null-sink approach where a crash strands the default.
//!
//! ## Real-time contract
//!
//! Both process callbacks are `RT_PROCESS`: no allocation, no locks, no logging.
//! The [`ProcessChain`] is built once on the control thread at the negotiated
//! rate and moved into the output callback; live params arrive via
//! `ArcSwap::load` once per block.
//!
//! ## Cannot be built or tested off-Linux
//!
//! `pipewire-sys` needs `libpipewire-0.3-dev` + `libclang` at build time and
//! `libpipewire-0.3.so.0` at runtime, so this file compiles only on the Linux
//! CI runner / the developer's Linux box — never on the macOS or Windows hosts.
//! The DSP seam it depends on ([`process_block`]) stays host-unit-tested.

#![cfg(target_os = "linux")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use hm_core::EngineState;
use hm_dsp::ProcessChain;
use pipewire as pw;
use pw::spa;

use crate::error::AudioError;
use crate::system_eq_shared::process_block;

/// Our virtual sink's node name — apps route here; also the value we exclude in
/// the mover so we never retarget our own output into ourselves.
const SINK_NAME: &str = "hypemuzik_eq";
/// Our output (playback-to-real-device) stream's node name.
const OUT_NAME: &str = "hypemuzik_eq_out";
/// Scheduling group shared by the sink and output streams. Nodes in one
/// `node.group` are driven by the **same** driver in the same cycle, so both our
/// streams run on one clock — the ring never drifts. This is exactly how
/// `pw-loopback` keeps its capture+playback pair sample-locked.
const NODE_GROUP: &str = "hypemuzik.eq";
const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Ring capacity in samples: generously larger than any plausible graph quantum
/// (8192 frames × 2ch) so the sink→output hop never overflows under jitter.
const RING_SAMPLES: usize = 8192 * CHANNELS * 4;
/// How long `start()` waits for the graph to prove it is driving our pipeline.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// PipeWire node `media.class` for an application playback stream — the streams
/// the mover retargets onto our sink.
const CLASS_OUTPUT_STREAM: &str = "Stream/Output/Audio";

/// Whether a live PipeWire server is reachable for this session.
///
/// Cheap probe: the well-known `$XDG_RUNTIME_DIR/pipewire-0` socket. We
/// deliberately avoid spinning up a full `Context`/`Core` just to answer
/// `available()`; [`PipewireSystemEq::start`] does the real connection and fails
/// honestly if the socket lies.
pub fn available() -> bool {
    socket_present()
}

/// True when the PipeWire client socket exists in the runtime dir (or the
/// explicit `PIPEWIRE_REMOTE` override names a reachable server).
fn socket_present() -> bool {
    if let Some(remote) = std::env::var_os("PIPEWIRE_REMOTE") {
        if !remote.is_empty() {
            return true;
        }
    }
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return false;
    };
    std::path::Path::new(&dir).join("pipewire-0").exists()
}

/// Message from `Drop` to the worker's main loop: tear down and quit.
enum Cmd {
    Stop,
}

/// A running PipeWire system-wide EQ. Everything PipeWire lives on the worker
/// thread; this handle only holds the terminate channel, the worker join handle,
/// and the shared liveness counters — so it is `Send` (auto-derived; no raw
/// PipeWire pointers cross the thread boundary).
pub struct PipewireSystemEq {
    sender: pw::channel::Sender<Cmd>,
    worker: Option<JoinHandle<()>>,
    /// Advanced every output-callback block; sampled by the engine watchdog.
    heartbeat: Arc<AtomicU64>,
}

impl PipewireSystemEq {
    /// Connect to PipeWire, stand up the two streams and the stream mover, and
    /// start processing. Returns only once the graph has confirmed it is driving
    /// the pipeline (or a real init error surfaces) — never a phantom "on".
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        if !available() {
            return Err(AudioError::Unavailable(
                "PipeWire is not running for this session".into(),
            ));
        }

        // The pw channel's Receiver is bound to the loop thread and is *not* Send,
        // so it must be created on the worker. The worker builds the channel as
        // its first act and hands the (Send) Sender back to us here.
        let (sender_tx, sender_rx) = mpsc::channel::<pw::channel::Sender<Cmd>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let heartbeat = Arc::new(AtomicU64::new(0));

        let hb = heartbeat.clone();
        let worker = std::thread::Builder::new()
            .name("hm-system-eq-pw".into())
            .spawn(move || run_loop(state, sender_tx, ready_tx, hb))
            .map_err(|e| AudioError::Stream(format!("system EQ worker: {e}")))?;

        // Without the terminate handle we can never stop the loop, so its absence
        // is a hard startup failure.
        let sender = match sender_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(s) => s,
            Err(_) => {
                let _ = worker.join();
                return Err(AudioError::Stream(
                    "PipeWire worker did not initialize".into(),
                ));
            }
        };

        match ready_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                sender,
                worker: Some(worker),
                heartbeat,
            }),
            Ok(Err(msg)) => {
                // The worker already tore its half down before reporting; just join.
                let _ = worker.join();
                Err(AudioError::Stream(msg))
            }
            Err(_) => {
                // Timed out without a verdict: unlike a CLI pipe, a silent-but-
                // connected graph still drives the output callback, so a healthy
                // pipeline reports fast. A timeout means the loop never came up —
                // treat as failure and tear down rather than show a false "on".
                let _ = sender.send(Cmd::Stop);
                let _ = worker.join();
                Err(AudioError::Stream(
                    "PipeWire pipeline did not start within the timeout".into(),
                ))
            }
        }
    }

    /// Current heartbeat value — for an external liveness watchdog. Frozen across
    /// samples while enabled means the graph stopped driving us.
    #[allow(dead_code)]
    pub fn heartbeat(&self) -> u64 {
        self.heartbeat.load(Ordering::Relaxed)
    }
}

impl Drop for PipewireSystemEq {
    fn drop(&mut self) {
        // Ask the loop to clear routing, restore state, and quit. If the channel
        // is already dead the worker has exited; either way we join.
        let _ = self.sender.send(Cmd::Stop);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Worker: the PipeWire main loop lives entirely here (all types are !Send).
// ---------------------------------------------------------------------------

/// Shared, single-threaded (loop-thread only) state for the stream mover. Held
/// behind `Rc<RefCell<…>>` so the registry callback and the Stop handler can both
/// reach it. Never touched off the loop thread.
#[derive(Default)]
struct Mover {
    /// The `default` metadata proxy, once discovered — how we set `target.object`.
    metadata: Option<pw::metadata::Metadata>,
    /// Our sink's `object.serial`, once its node appears. `target.object` values
    /// are serials (matched exactly by WirePlumber, like EasyEffects).
    sink_serial: Option<String>,
    /// App stream node ids we have retargeted (cleared on teardown).
    moved: Vec<u32>,
    /// App stream node ids seen before we knew the sink serial / metadata.
    pending: Vec<u32>,
}

impl Mover {
    /// Retarget one app stream node onto our sink, if we can now. Returns true if
    /// the move was issued (or already tracked); false if it must stay pending.
    fn try_move(&mut self, node_id: u32) -> bool {
        let (Some(meta), Some(serial)) = (self.metadata.as_ref(), self.sink_serial.as_ref()) else {
            return false;
        };
        // Spa:Id + the sink's object.serial — the exact routing metadata
        // WirePlumber honours (`linking.allow-moving-streams`).
        meta.set_property(node_id, "target.object", Some("Spa:Id"), Some(serial));
        if !self.moved.contains(&node_id) {
            self.moved.push(node_id);
        }
        true
    }

    /// Flush any streams that were waiting on the sink serial / metadata.
    fn flush_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for id in pending {
            if !self.try_move(id) {
                self.pending.push(id);
            }
        }
    }

    /// Clear `target.object` on every stream we moved, so apps snap back to the
    /// real device on teardown and no stale routing metadata is left behind.
    fn clear_all(&mut self) {
        if let Some(meta) = self.metadata.as_ref() {
            for id in self.moved.drain(..) {
                meta.set_property(id, "target.object", None, None);
            }
        } else {
            self.moved.clear();
        }
    }
}

/// The worker body: owns the main loop, both streams, the registry listener, and
/// the terminate receiver for the lifetime of the session.
fn run_loop(
    state: Arc<ArcSwap<EngineState>>,
    sender_tx: mpsc::Sender<pw::channel::Sender<Cmd>>,
    ready: mpsc::Sender<Result<(), String>>,
    heartbeat: Arc<AtomicU64>,
) {
    // Create the pw channel here so its loop-bound (!Send) Receiver never leaves
    // this thread; hand the Send half back to `start()`.
    let (pw_sender, pw_receiver) = pw::channel::channel::<Cmd>();
    if sender_tx.send(pw_sender).is_err() {
        return; // the control thread gave up already
    }
    match build_and_run(state, pw_receiver, &ready, heartbeat) {
        Ok(()) => {}
        Err(e) => {
            // Report the init failure to `start()` (idempotent: if we already
            // sent Ok, this extra Err is ignored by the bounded handshake).
            let _ = ready.send(Err(e));
        }
    }
}

fn build_and_run(
    state: Arc<ArcSwap<EngineState>>,
    receiver: pw::channel::Receiver<Cmd>,
    ready: &mpsc::Sender<Result<(), String>>,
    heartbeat: Arc<AtomicU64>,
) -> Result<(), String> {
    // Best-effort RT scheduling + denormal flush for the thread the graph will
    // call our process callbacks on (pipewire runs them on the data loop, but
    // promoting this thread also covers the ring hand-off).
    crate::thread_util::promote_current_thread_to_realtime();
    crate::thread_util::enable_denormal_flush_once();

    let main_loop = pw::main_loop::MainLoopRc::new(None).map_err(|e| format!("main loop: {e}"))?;
    let context =
        pw::context::ContextRc::new(&main_loop, None).map_err(|e| format!("context: {e}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|e| format!("connect to PipeWire: {e}"))?;
    let registry = core
        .get_registry_rc()
        .map_err(|e| format!("get registry: {e}"))?;

    // Lock-free hand-off from the sink callback (producer) to the output callback
    // (consumer). Both callbacks run on the graph's data loop.
    let ring = rtrb::RingBuffer::<f32>::new(RING_SAMPLES);
    let mut producer = ring.0;
    let mut consumer = ring.1;

    // --- Sink stream: the virtual sink apps route into ------------------------
    let sink_props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Duplex",
        *pw::keys::MEDIA_CLASS => "Audio/Sink",
        *pw::keys::NODE_NAME => SINK_NAME,
        *pw::keys::NODE_DESCRIPTION => "HypeMuzik EQ",
        // Mark as virtual so UIs group it as an effect sink, not a device.
        "node.virtual" => "true",
        // Same driver/clock as the output stream → the ring never drifts.
        "node.group" => NODE_GROUP,
        *pw::keys::AUDIO_CHANNELS => "2",
    };
    let sink_stream =
        pw::stream::Stream::new(&core, SINK_NAME, sink_props).map_err(|e| format!("sink: {e}"))?;

    let _sink_listener = sink_stream
        .add_local_listener::<()>()
        .process(move |stream, _| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    let n_bytes = data.chunk().size() as usize;
                    if let Some(slice) = data.data() {
                        let valid = &slice[..n_bytes.min(slice.len())];
                        // Interleaved f32 LE → push samples into the ring. Drop on
                        // overflow (never block the RT callback).
                        for chunk in valid.chunks_exact(4) {
                            let s = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                            let _ = producer.push(s);
                        }
                    }
                }
            }
        })
        .register();

    // --- Output stream: renders the processed audio to the real device --------
    let out_props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_CLASS => CLASS_OUTPUT_STREAM,
        *pw::keys::MEDIA_ROLE => "DSP",
        *pw::keys::NODE_NAME => OUT_NAME,
        *pw::keys::NODE_DESCRIPTION => "HypeMuzik EQ output",
        *pw::keys::APP_NAME => "HypeMuzik",
        // Same group as the sink so both share one driver/clock (see NODE_GROUP).
        "node.group" => NODE_GROUP,
        *pw::keys::AUDIO_CHANNELS => "2",
    };
    let out_stream =
        pw::stream::Stream::new(&core, OUT_NAME, out_props).map_err(|e| format!("output: {e}"))?;

    // Built once, at the fixed negotiated rate, and moved into the RT callback.
    let mut chain = ProcessChain::standard(RATE as f32, CHANNELS);
    // Reused every callback so the RT path never allocates or zeroes a big stack
    // array; sized well above any plausible quantum (8192 frames × 2ch).
    let mut scratch = vec![0f32; 8192 * CHANNELS];
    let out_ready = ready.clone();
    let hb = heartbeat.clone();
    let mut announced = false;
    let _out_listener = out_stream
        .add_local_listener::<()>()
        .state_changed({
            let err_ready = ready.clone();
            move |_stream, _, _old, new| {
                if let pw::stream::StreamState::Error(msg) = new {
                    let _ = err_ready.send(Err(format!("output stream error: {msg}")));
                }
            }
        })
        .process(move |stream, _| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    let stride = CHANNELS * 4;
                    let max_bytes = data.data().map(|s| s.len()).unwrap_or(0);
                    let max_samples = max_bytes / 4;
                    // Pop as many whole frames as the buffer holds; silence-fill
                    // the tail on ring underrun so the device never starves.
                    let want = max_samples.min(scratch.len());
                    for s in scratch.iter_mut().take(want) {
                        *s = consumer.pop().unwrap_or(0.0);
                    }
                    process_block(&mut chain, &mut scratch[..want], CHANNELS, &state.load());
                    if let Some(slice) = data.data() {
                        for (out, s) in slice.chunks_exact_mut(4).zip(scratch[..want].iter()) {
                            out.copy_from_slice(&s.to_le_bytes());
                        }
                    }
                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = stride as i32;
                    *chunk.size_mut() = (want * 4) as u32;
                }
            }
            hb.fetch_add(1, Ordering::Relaxed);
            if !announced {
                let _ = out_ready.send(Ok(()));
                announced = true;
            }
        })
        .register();

    // --- Connect both streams with a fixed F32/48k/stereo format --------------
    let sink_bytes = audio_format_pod();
    let mut sink_params = [spa::pod::Pod::from_bytes(&sink_bytes)
        .ok_or("could not build sink format pod")?];
    sink_stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
            &mut sink_params,
        )
        .map_err(|e| format!("connect sink: {e}"))?;

    let out_bytes = audio_format_pod();
    let mut out_params =
        [spa::pod::Pod::from_bytes(&out_bytes).ok_or("could not build output format pod")?];
    out_stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut out_params,
        )
        .map_err(|e| format!("connect output: {e}"))?;

    // --- Stream mover: retarget app streams onto our sink ---------------------
    let mover = Rc::new(RefCell::new(Mover::default()));
    let registry_for_cb = registry.clone();
    let mover_cb = mover.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |obj| handle_global(&registry_for_cb, &mover_cb, obj))
        .global_remove({
            let mover_cb = mover.clone();
            move |id| {
                let mut m = mover_cb.borrow_mut();
                m.moved.retain(|&x| x != id);
                m.pending.retain(|&x| x != id);
            }
        })
        .register();

    // --- Terminate handler: clear routing, then quit --------------------------
    let quit_loop = main_loop.clone();
    let quit_mover = mover.clone();
    let _receiver = receiver.attach(main_loop.loop_(), move |cmd| match cmd {
        Cmd::Stop => {
            quit_mover.borrow_mut().clear_all();
            quit_loop.quit();
        }
    });

    main_loop.run();
    // Loop returned: streams/registry/core drop here → PipeWire destroys our
    // client objects and WirePlumber re-routes any still-moved app back to the
    // real device.
    Ok(())
}

/// Registry `global` handler: discover the `default` metadata, learn our sink's
/// serial, and retarget every app playback stream onto our sink.
fn handle_global(
    registry: &pw::registry::RegistryRc,
    mover: &Rc<RefCell<Mover>>,
    obj: &pw::registry::GlobalObject<&spa::utils::dict::DictRef>,
) {
    let props = obj.props;
    match obj.type_ {
        pw::types::ObjectType::Metadata => {
            let is_default = props
                .and_then(|p| p.get("metadata.name"))
                .map(|n| n == "default")
                .unwrap_or(false);
            if is_default {
                let bound: Result<pw::metadata::Metadata, _> = registry.bind(obj);
                if let Ok(meta) = bound {
                    let mut m = mover.borrow_mut();
                    m.metadata = Some(meta);
                    m.flush_pending();
                }
            }
        }
        pw::types::ObjectType::Node => {
            let node_name = props.and_then(|p| p.get(*pw::keys::NODE_NAME));
            let media_class = props.and_then(|p| p.get(*pw::keys::MEDIA_CLASS));

            // Our own sink node: capture its serial so the mover can target it.
            if node_name == Some(SINK_NAME) {
                if let Some(serial) = props.and_then(|p| p.get("object.serial")) {
                    let mut m = mover.borrow_mut();
                    m.sink_serial = Some(serial.to_string());
                    m.flush_pending();
                }
                return;
            }

            // App playback streams: retarget onto our sink, unless it's our own
            // output or HypeMuzik's own playback (macOS-parity exclusion).
            if is_moveable_output(media_class) {
                let app_name = props.and_then(|p| p.get(*pw::keys::APP_NAME));
                let app_binary = props.and_then(|p| p.get("application.process.binary"));
                if is_own_stream(node_name, app_name, app_binary) {
                    return;
                }
                let mut m = mover.borrow_mut();
                if !m.try_move(obj.id) {
                    m.pending.push(obj.id);
                }
            }
        }
        _ => {}
    }
}

/// A node's `media.class` marks it as an application playback stream (the kind we
/// route onto our sink).
fn is_moveable_output(media_class: Option<&str>) -> bool {
    media_class == Some(CLASS_OUTPUT_STREAM)
}

/// Whether a playback stream belongs to HypeMuzik (our own EQ output, or the
/// app's own music playback) and must therefore be excluded from re-routing —
/// otherwise our output would feed back into our sink, and the app's audio would
/// be processed twice. Pure so it is unit-tested on the macOS host.
fn is_own_stream(
    node_name: Option<&str>,
    app_name: Option<&str>,
    app_binary: Option<&str>,
) -> bool {
    if node_name == Some(OUT_NAME) || node_name == Some(SINK_NAME) {
        return true;
    }
    let hay = |s: Option<&str>| s.map(|v| v.to_ascii_lowercase()).unwrap_or_default();
    hay(app_name).contains("hypemuzik") || hay(app_binary).contains("hypemuzik")
}

/// Serialize a fixed **F32LE / 48 kHz / stereo (FL,FR)** audio format into a SPA
/// `EnumFormat` pod. Both streams request this; PipeWire's adapters convert and
/// resample every app to/from it, so the DSP always sees a known layout.
fn audio_format_pod() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(CHANNELS as u32);
    let mut position = [0u32; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);

    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .expect("serialize audio format pod")
    .0
    .into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moveable_only_matches_output_streams() {
        assert!(is_moveable_output(Some("Stream/Output/Audio")));
        assert!(!is_moveable_output(Some("Stream/Input/Audio")));
        assert!(!is_moveable_output(Some("Audio/Sink")));
        assert!(!is_moveable_output(None));
    }

    #[test]
    fn excludes_our_own_nodes_by_name() {
        assert!(is_own_stream(Some(OUT_NAME), None, None));
        assert!(is_own_stream(Some(SINK_NAME), None, None));
    }

    #[test]
    fn excludes_hypemuzik_by_app_identity() {
        assert!(is_own_stream(Some("Firefox"), Some("HypeMuzik"), None));
        assert!(is_own_stream(None, Some("hypemuzik"), None));
        assert!(is_own_stream(None, None, Some("/usr/bin/hypemuzik")));
    }

    #[test]
    fn does_not_exclude_other_apps() {
        assert!(!is_own_stream(Some("Firefox"), Some("Firefox"), Some("/usr/lib/firefox/firefox")));
        assert!(!is_own_stream(Some("Spotify"), Some("Spotify"), None));
    }
}
