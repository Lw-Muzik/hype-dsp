//! MilkDrop visualizer: spawn the standalone sidecar window and stream the
//! engine's post-DSP PCM to it.
//!
//! The renderer is a separate process (`hm-visualizer`) so its OpenGL window has
//! its own main-thread event loop (required on macOS) and a projectM crash can't
//! take the app down. We pipe the engine's lock-free mono waveform tap to the
//! sidecar's stdin at a modest rate — no audio-thread work, no large IPC.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use hm_audio::AudioEngine;
use hm_core::IpcError;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};

/// How often the waveform is pushed to the sidecar (projectM interpolates).
const PCM_FPS: u64 = 30;

/// How often the engine's waveform is pushed to the embedded (webview)
/// butterchurn visualizer over the `visualizer:pcm` event. A touch above 30 so
/// the canvas — which renders on its own rAF loop and reads the latest frame —
/// always has fresh-ish audio.
const WEB_PCM_FPS: u64 = 45;

/// Managed handle to the running visualizer process (if any).
#[derive(Default)]
pub struct VisualizerState {
    inner: Mutex<Option<Running>>,
}

struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

impl Running {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        if let Some(p) = self.pump.take() {
            let _ = p.join();
        }
        let _ = self.child.wait();
    }
}

/// Path to the bundled sidecar binary — next to the app executable in a packaged
/// build, or the shared `target/<profile>` dir during development.
fn sidecar_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "hm-visualizer.exe"
    } else {
        "hm-visualizer"
    };
    // Packaged build: bundled next to the app executable.
    let p = dir.join(name);
    if p.exists() {
        return Some(p);
    }
    // Dev: `tauri dev` runs the app from target/debug, but the sidecar only
    // builds in release — look in the sibling profile dirs too.
    let target = dir.parent()?;
    ["release", "debug"]
        .iter()
        .map(|profile| target.join(profile).join(name))
        .find(|q| q.exists())
}

/// The bundled `.milk` preset directory: a packaged resource, or the crate's
/// `presets/` dir during development. Empty when neither is found (projectM then
/// shows its built-in idle preset).
fn preset_dir(app: &AppHandle) -> String {
    if let Ok(p) = app.path().resolve("presets", BaseDirectory::Resource) {
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    // Dev: target/<profile>/<app> → up to the workspace root → crate presets.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.ancestors().nth(3) {
            let p = root.join("crates/hm-visualizer/presets");
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    String::new()
}

/// Whether the native visualizer sidecar is present in this build.
#[tauri::command]
pub fn visualizer_available() -> bool {
    sidecar_path().is_some()
}

/// Open the MilkDrop visualizer window and start streaming audio to it. Replaces
/// any window already open.
#[tauri::command]
pub fn visualizer_start(
    app: AppHandle,
    engine: State<'_, AudioEngine>,
    state: State<'_, VisualizerState>,
    fps: Option<i32>,
    beat: Option<f32>,
    preset_secs: Option<f64>,
) -> Result<(), IpcError> {
    if let Some(prev) = state.inner.lock().expect("visualizer poisoned").take() {
        prev.shutdown();
    }
    let bin =
        sidecar_path().ok_or_else(|| IpcError::new("unavailable", "Visualizer isn't available."))?;

    let mut child = Command::new(bin)
        .arg(preset_dir(&app))
        .arg(fps.unwrap_or(30).to_string())
        .arg(beat.unwrap_or(1.0).to_string())
        .arg(preset_secs.unwrap_or(20.0).to_string())
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| IpcError::new("spawn", format!("couldn't start visualizer: {e}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| IpcError::new("spawn", "no stdin pipe to the visualizer"))?;

    let tap = engine.spectrum();
    let stop = Arc::new(AtomicBool::new(false));
    let run = stop.clone();
    let pump = std::thread::Builder::new()
        .name("hm-viz-pcm".into())
        .spawn(move || {
            let period = Duration::from_millis(1000 / PCM_FPS);
            let mut bytes = Vec::with_capacity(2048);
            while !run.load(Ordering::Relaxed) {
                bytes.clear();
                for s in tap.load_waveform() {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                // The window closing breaks the pipe — stop quietly.
                if stdin.write_all(&bytes).is_err() {
                    break;
                }
                std::thread::sleep(period);
            }
        })
        .ok();

    *state.inner.lock().expect("visualizer poisoned") = Some(Running { child, stop, pump });
    Ok(())
}

/// Close the visualizer window and stop streaming.
#[tauri::command]
pub fn visualizer_stop(state: State<'_, VisualizerState>) {
    if let Some(r) = state.inner.lock().expect("visualizer poisoned").take() {
        r.shutdown();
    }
}

/* ----------------------------------------------------- embedded (webview) PCM */

/// Managed handle to the embedded-visualizer PCM emitter thread. Independent of
/// the native sidecar window above: this feeds the in-app butterchurn canvas via
/// `visualizer:pcm` events, and runs only while that view is mounted.
#[derive(Default)]
pub struct PcmStreamState {
    inner: Mutex<Option<PcmPump>>,
}

struct PcmPump {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PcmPump {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Start emitting the engine's latest mono waveform to the webview over
/// `visualizer:pcm` (a `Vec<f32>`), for the embedded butterchurn visualizer.
/// Idempotent — replaces any stream already running.
#[tauri::command]
pub fn visualizer_pcm_start(
    app: AppHandle,
    engine: State<'_, AudioEngine>,
    state: State<'_, PcmStreamState>,
) {
    if let Some(prev) = state.inner.lock().expect("pcm stream poisoned").take() {
        prev.shutdown();
    }
    let tap = engine.spectrum();
    let stop = Arc::new(AtomicBool::new(false));
    let run = stop.clone();
    let handle = std::thread::Builder::new()
        .name("hm-viz-web-pcm".into())
        .spawn(move || {
            let period = Duration::from_millis(1000 / WEB_PCM_FPS);
            while !run.load(Ordering::Relaxed) {
                // Only serialization can fail here (no listener ≠ error), so the
                // frontend is responsible for calling stop on unmount.
                if app.emit("visualizer:pcm", tap.load_waveform()).is_err() {
                    break;
                }
                std::thread::sleep(period);
            }
        })
        .ok();
    *state.inner.lock().expect("pcm stream poisoned") = Some(PcmPump { stop, handle });
}

/// Stop the embedded visualizer's PCM stream.
#[tauri::command]
pub fn visualizer_pcm_stop(state: State<'_, PcmStreamState>) {
    if let Some(p) = state.inner.lock().expect("pcm stream poisoned").take() {
        p.shutdown();
    }
}
