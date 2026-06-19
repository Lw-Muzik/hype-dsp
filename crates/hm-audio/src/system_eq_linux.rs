//! System-wide EQ on Linux via a PulseAudio / PipeWire virtual sink.
//!
//! Unlike macOS (process taps that mute the originals in place), the portable
//! Linux approach is to *re-route*: create a null sink, make it the default
//! output so every app renders into it, capture its `.monitor`, run the samples
//! through the shared DSP [`ProcessChain`] (with the engine's live params), and
//! play the result to the **real** output device. The originals never reach the
//! speakers (they go to the null sink), so there's no doubling — this is exactly
//! how EasyEffects' "process all outputs" works.
//!
//! It drives the ubiquitous `pactl` / `parec` / `pacat` CLIs (present wherever
//! PulseAudio or PipeWire's pulse layer is), so it needs **no extra crates** and
//! works on both stacks. On stop (Drop) it restores the previous default sink
//! and unloads the null sink.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use hm_core::EngineState;
use hm_dsp::ProcessChain;

use crate::error::AudioError;

const SINK_NAME: &str = "hypemuzik_eq";
const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Frames per processing block (~21 ms at 48 kHz) — small enough for responsive
/// EQ, large enough to keep CLI piping cheap.
const BLOCK_FRAMES: usize = 1024;

/// Whether `pactl` is present and a PulseAudio/PipeWire server is reachable.
pub fn available() -> bool {
    Command::new("pactl")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A running system-wide EQ pipeline. Dropping it tears everything down and
/// restores the previous default sink.
pub struct LinuxSystemEq {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    parec: Child,
    pacat: Child,
    module_id: String,
    previous_default: String,
}

impl LinuxSystemEq {
    /// Stand up the virtual sink, route audio into it, and start processing.
    /// `state` is the engine's live parameter handle (EQ/effects/power/volume).
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        if !available() {
            return Err(AudioError::Unavailable(
                "PulseAudio/PipeWire (pactl) is not available".into(),
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

        if let Err(e) = set_default_sink(SINK_NAME) {
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
        let mut parec = parec;
        let mut pacat = pacat;
        let worker = match std::thread::Builder::new()
            .name("hm-system-eq".into())
            .spawn(move || process_loop(state, run, rx, tx))
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

impl Drop for LinuxSystemEq {
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
fn process_loop(
    state: Arc<ArcSwap<EngineState>>,
    run: Arc<AtomicBool>,
    mut rx: impl Read,
    mut tx: impl Write,
) {
    let mut chain = ProcessChain::standard(RATE as f32, CHANNELS);
    let mut bytes = vec![0u8; BLOCK_FRAMES * CHANNELS * 4];
    let mut samples = vec![0f32; BLOCK_FRAMES * CHANNELS];

    while run.load(Ordering::Relaxed) {
        if rx.read_exact(&mut bytes).is_err() {
            break;
        }
        for (s, chunk) in samples.iter_mut().zip(bytes.chunks_exact(4)) {
            *s = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        let st = state.load();
        if (st.master_volume - 1.0).abs() > f32::EPSILON {
            for s in samples.iter_mut() {
                *s *= st.master_volume;
            }
        }
        if st.power {
            chain.set_params(&st);
            chain.process(&mut samples, CHANNELS);
        }

        for (s, chunk) in samples.iter().zip(bytes.chunks_exact_mut(4)) {
            chunk.copy_from_slice(&s.to_le_bytes());
        }
        if tx.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = tx.flush();
}

/// Spawn `parec` (monitor capture) + `pacat` (playback to the real sink) and
/// hand back the processes plus their stdio ends.
fn spawn_pipeline(
    real_sink: &str,
) -> Result<(Child, Child, ChildStdout, ChildStdin), AudioError> {
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

fn set_default_sink(name: &str) -> Result<(), AudioError> {
    Command::new("pactl")
        .args(["set-default-sink", name])
        .status()
        .map_err(|e| AudioError::Stream(format!("pactl set-default-sink: {e}")))?;
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
