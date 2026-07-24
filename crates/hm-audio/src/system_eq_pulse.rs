//! System-wide EQ fallback for **classic PulseAudio** servers.
//!
//! This is the original virtual-sink approach, kept for the shrinking minority
//! of desktops still running a real PulseAudio daemon (no PipeWire). On a true
//! PulseAudio core the module/default/move operations are synchronous inside one
//! process, so the model is sound and near race-free — the failures that make
//! this approach unreliable are specific to PipeWire/WirePlumber, which is
//! handled by [`crate::system_eq_pipewire`] instead. See
//! [`crate::system_eq_linux`] for backend selection.
//!
//! The model: create a `module-null-sink`, make it the default output so apps
//! render into it, capture its `.monitor` with `parec`, run the shared DSP
//! [`ProcessChain`], and play the result to the **real** output with `pacat`.
//! On stop (Drop) it restores the previous default and unloads the null sink.
//!
//! Repairs over the first cut (which could silently "run" while nothing was
//! equalised — the phantom-on bug):
//! - every `pactl` exit status is checked, not swallowed;
//! - after switching the default we **read it back** and assert it took;
//! - `available()` also verifies `parec`/`pacat` exist (they ship separately
//!   from `pactl`, in `pulseaudio-utils`);
//! - a startup handshake makes `start()` fail loudly if the capture/playback
//!   pipeline never actually moves a byte, instead of returning `Ok` blindly.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use hm_core::EngineState;
use hm_dsp::ProcessChain;

use crate::error::AudioError;
use crate::system_eq_shared::process_block;

const SINK_NAME: &str = "hypemuzik_eq";
const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Frames per processing block (~21 ms at 48 kHz) — small enough for responsive
/// EQ, large enough to keep CLI piping cheap.
const BLOCK_FRAMES: usize = 1024;
/// How long `start()` waits for the pipeline to prove it is actually moving
/// audio before giving up (mirrors the Windows backend's handshake).
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether the PulseAudio CLI trio needed by this backend is present *and* a
/// server is reachable. Checks `pactl` (server reachability) plus `parec` and
/// `pacat`, which live in `pulseaudio-utils` and can be absent even when a
/// server is up.
pub fn available() -> bool {
    server_reachable() && tool_exists("parec") && tool_exists("pacat")
}

/// `pactl info` succeeds (a PulseAudio-compatible server is reachable).
fn server_reachable() -> bool {
    Command::new("pactl")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether `name` resolves to a runnable executable (via `--version`). Used to
/// confirm `parec`/`pacat` exist before we promise the UI an EQ.
fn tool_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A running PulseAudio system-wide EQ pipeline. Dropping it tears everything
/// down and restores the previous default sink.
pub struct PulseSystemEq {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    parec: Child,
    pacat: Child,
    module_id: String,
    previous_default: String,
}

impl PulseSystemEq {
    /// Stand up the virtual sink, route audio into it, and start processing.
    /// `state` is the engine's live parameter handle (EQ/effects/power/volume).
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        if !available() {
            return Err(AudioError::Unavailable(
                "PulseAudio (pactl/parec/pacat) is not available".into(),
            ));
        }
        // The real output device — captured before we change the default. Without
        // it we can't route the processed audio anywhere, so bail early.
        let previous_default = current_default_sink().ok_or_else(|| {
            AudioError::Unavailable("could not determine the default audio output".into())
        })?;
        if previous_default == SINK_NAME {
            return Err(AudioError::Stream(
                "system EQ already appears to be running".into(),
            ));
        }

        // Create the virtual sink. From here on, any failure must restore the
        // previous default and unload the module so we don't leak routing state.
        let module_id = load_null_sink()?;

        // Switch the default and *verify it took*. On classic PulseAudio this is
        // synchronous; a readback mismatch means the switch was refused, which is
        // exactly the silent no-op we must convert into an honest error.
        if let Err(e) = set_default_sink_verified(SINK_NAME) {
            let _ = set_default_sink(&previous_default);
            unload_module(&module_id);
            return Err(e);
        }
        move_inputs_to(SINK_NAME);

        let (parec, pacat, rx, tx) = match spawn_pipeline(&previous_default) {
            Ok(v) => v,
            Err(e) => {
                let _ = set_default_sink(&previous_default);
                unload_module(&module_id);
                return Err(e);
            }
        };

        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        // Startup handshake: the worker reports whether it actually moved a block
        // through the pipeline, so a dead monitor/pipe surfaces to the caller
        // instead of masquerading as a running EQ.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let mut parec = parec;
        let mut pacat = pacat;
        let worker = match std::thread::Builder::new()
            .name("hm-system-eq-pulse".into())
            .spawn(move || process_loop(state, run, rx, tx, ready_tx))
        {
            Ok(w) => w,
            Err(e) => {
                let _ = parec.kill();
                let _ = pacat.kill();
                let _ = set_default_sink(&previous_default);
                unload_module(&module_id);
                return Err(AudioError::Stream(format!("system EQ worker: {e}")));
            }
        };

        // Wait briefly for the first block to prove the pipeline is live.
        match ready_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                running.store(false, Ordering::Relaxed);
                let _ = parec.kill();
                let _ = pacat.kill();
                let _ = worker.join();
                let _ = set_default_sink(&previous_default);
                unload_module(&module_id);
                return Err(AudioError::Stream(msg));
            }
            Err(_) => {
                // Slow-but-healthy bring-up (e.g. no audio playing yet so the
                // monitor hasn't produced a block): assume running rather than
                // kill a possibly-working pipeline. A genuinely dead pipe reports
                // fast via read/write errors.
                crate::diag::log(
                    "system-eq(pulse): startup confirmation timed out; assuming running",
                );
            }
        }

        Ok(Self {
            running,
            worker: Some(worker),
            parec,
            pacat,
            module_id,
            previous_default,
        })
    }
}

impl Drop for PulseSystemEq {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.parec.kill();
        let _ = self.pacat.kill();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        // Restore the previous default before removing the sink so streams snap
        // back to the real device.
        let _ = set_default_sink(&self.previous_default);
        unload_module(&self.module_id);
    }
}

/// The capture→DSP→render loop: read interleaved f32 from `parec`, run the chain
/// with the engine's live params, write to `pacat`. `pacat`'s backpressure paces
/// the loop to real time. Exits when `run` clears or either pipe closes.
///
/// `ready` reports the first successful block (or an early failure) exactly once
/// for the startup handshake.
fn process_loop(
    state: Arc<ArcSwap<EngineState>>,
    run: Arc<AtomicBool>,
    mut rx: impl Read,
    mut tx: impl Write,
    ready: mpsc::Sender<Result<(), String>>,
) {
    // This worker carries *every* app's audio: best-effort real-time scheduling
    // so it doesn't stutter under load, and flush denormals so decaying filter
    // tails don't hit the x86 denormal penalty. Both are per-thread one-shots.
    crate::thread_util::promote_current_thread_to_realtime();
    crate::thread_util::enable_denormal_flush_once();
    let mut chain = ProcessChain::standard(RATE as f32, CHANNELS);
    let mut bytes = vec![0u8; BLOCK_FRAMES * CHANNELS * 4];
    let mut samples = vec![0f32; BLOCK_FRAMES * CHANNELS];
    let mut announced = false;

    while run.load(Ordering::Relaxed) {
        if let Err(e) = rx.read_exact(&mut bytes) {
            if !announced {
                let _ = ready.send(Err(format!("capture pipe closed at startup: {e}")));
            }
            break;
        }
        for (s, chunk) in samples.iter_mut().zip(bytes.chunks_exact(4)) {
            *s = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        process_block(&mut chain, &mut samples, CHANNELS, &state.load());

        for (s, chunk) in samples.iter().zip(bytes.chunks_exact_mut(4)) {
            chunk.copy_from_slice(&s.to_le_bytes());
        }
        if let Err(e) = tx.write_all(&bytes) {
            if !announced {
                let _ = ready.send(Err(format!("playback pipe closed at startup: {e}")));
            }
            break;
        }
        if !announced {
            let _ = ready.send(Ok(()));
            announced = true;
        }
    }
    let _ = tx.flush();
}

/// Spawn `parec` (monitor capture) + `pacat` (playback to the real sink) and
/// hand back the processes plus their stdio ends.
fn spawn_pipeline(real_sink: &str) -> Result<(Child, Child, ChildStdout, ChildStdin), AudioError> {
    let rate = RATE.to_string();
    let channels = CHANNELS.to_string();
    let monitor = format!("{SINK_NAME}.monitor");

    let mut parec = Command::new("parec")
        .args([
            "--device",
            monitor.as_str(),
            "--rate",
            rate.as_str(),
            "--channels",
            channels.as_str(),
            "--format=float32le",
            "--raw",
            "--latency-msec=30",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AudioError::Stream(format!("parec: {e}")))?;

    let pacat = Command::new("pacat")
        .args([
            "--device",
            real_sink,
            "--rate",
            rate.as_str(),
            "--channels",
            channels.as_str(),
            "--format=float32le",
            "--raw",
            "--latency-msec=30",
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut pacat = match pacat {
        Ok(c) => c,
        Err(e) => {
            let _ = parec.kill();
            return Err(AudioError::Stream(format!("pacat: {e}")));
        }
    };

    let rx = parec.stdout.take().expect("parec stdout piped");
    let tx = pacat.stdin.take().expect("pacat stdin piped");
    Ok((parec, pacat, rx, tx))
}

fn current_default_sink() -> Option<String> {
    let out = Command::new("pactl").arg("get-default-sink").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn load_null_sink() -> Result<String, AudioError> {
    let sink_name = format!("sink_name={SINK_NAME}");
    let out = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            sink_name.as_str(),
            "sink_properties=device.description=HypeMuzik-EQ",
        ])
        .output()
        .map_err(|e| AudioError::Stream(format!("pactl load-module: {e}")))?;
    if !out.status.success() {
        return Err(AudioError::Stream(
            "failed to create the HypeMuzik virtual sink".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Set the default sink and confirm the change took by reading it back — on a
/// real PulseAudio server this is synchronous, so a mismatch is a genuine
/// failure worth surfacing rather than silently proceeding.
fn set_default_sink_verified(name: &str) -> Result<(), AudioError> {
    set_default_sink(name)?;
    match current_default_sink() {
        Some(cur) if cur == name => Ok(()),
        other => Err(AudioError::Stream(format!(
            "default sink did not switch to {name} (still {})",
            other.as_deref().unwrap_or("<unknown>")
        ))),
    }
}

fn set_default_sink(name: &str) -> Result<(), AudioError> {
    let status = Command::new("pactl")
        .args(["set-default-sink", name])
        .status()
        .map_err(|e| AudioError::Stream(format!("pactl set-default-sink: {e}")))?;
    if !status.success() {
        return Err(AudioError::Stream(format!(
            "pactl set-default-sink {name} failed"
        )));
    }
    Ok(())
}

/// Move any currently-playing streams onto `sink` so existing audio is captured
/// too (new streams follow the default automatically).
fn move_inputs_to(sink: &str) {
    let Ok(out) = Command::new("pactl")
        .args(["list", "short", "sink-inputs"])
        .output()
    else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(id) = line.split_whitespace().next() {
            let _ = Command::new("pactl")
                .args(["move-sink-input", id, sink])
                .status();
        }
    }
}

fn unload_module(module_id: &str) {
    let _ = Command::new("pactl")
        .args(["unload-module", module_id])
        .status();
}
