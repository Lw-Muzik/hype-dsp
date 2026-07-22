//! MilkDrop visualizer: spawn the standalone sidecar window, stream the engine's
//! post-DSP PCM to it, and drive its preset selection from the app.
//!
//! The renderer is a separate process (`hm-visualizer`) so its OpenGL window has
//! its own main-thread event loop (required on macOS) and a projectM crash can't
//! take the app down. One stdin pipe carries both audio and control via a tiny
//! tagged protocol: `b'P'` + PCM frame, `b'L'` + preset name (see the sidecar).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use hm_audio::AudioEngine;
use hm_core::IpcError;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

/// How often the waveform is pushed to the sidecar (projectM interpolates).
const PCM_FPS: u64 = 30;

/// Managed handle to the running visualizer process (if any).
#[derive(Default)]
pub struct VisualizerState {
    inner: Mutex<Option<Running>>,
}

struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
    /// Shared so both the PCM pump and `set_preset` can write to the one pipe.
    stdin: Arc<Mutex<ChildStdin>>,
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
    let p = dir.join(name);
    if p.exists() {
        return Some(p);
    }
    // Dev: the app runs from target/debug but the sidecar only builds in
    // release — look in the sibling profile dirs too.
    let target = dir.parent()?;
    ["release", "debug"]
        .iter()
        .map(|profile| target.join(profile).join(name))
        .find(|q| q.exists())
}

/// The bundled `.milk` preset directory: a packaged resource, or the crate's
/// `presets/` dir during development. Empty when neither is found.
fn preset_dir(app: &AppHandle) -> String {
    if let Ok(p) = app.path().resolve("presets", BaseDirectory::Resource) {
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
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

/// Every bundled `.milk` preset name (file stem), sorted — the list the app's
/// Visuals view browses and drives the window with. `async` so scanning the
/// ~550-file preset directory runs on a worker thread, not the webview thread.
#[tauri::command(async)]
pub fn visualizer_preset_names(app: AppHandle) -> Vec<String> {
    let dir = preset_dir(&app);
    let mut names = Vec::new();
    if !dir.is_empty() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("milk") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        names.push(stem.to_owned());
                    }
                }
            }
        }
    }
    names.sort_by_key(|s| s.to_lowercase());
    names
}

/// Open the MilkDrop visualizer window, streaming audio to it and starting on
/// `preset` (a `.milk` file stem) when given. Replaces any window already open.
#[tauri::command]
pub fn visualizer_start(
    app: AppHandle,
    engine: State<'_, AudioEngine>,
    state: State<'_, VisualizerState>,
    fps: Option<i32>,
    beat: Option<f32>,
    preset_secs: Option<f64>,
    preset: Option<String>,
) -> Result<(), IpcError> {
    if let Some(prev) = state.inner.lock().expect("visualizer poisoned").take() {
        prev.shutdown();
    }
    let bin =
        sidecar_path().ok_or_else(|| IpcError::new("unavailable", "Visualizer isn't available."))?;

    // `CREATE_NO_WINDOW` (Windows): suppress the sidecar's console window —
    // its SDL/GL window is a real window and is unaffected by this flag.
    #[allow(unused_mut)]
    let mut cmd = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .arg(preset_dir(&app))
        .arg(fps.unwrap_or(30).to_string())
        .arg(beat.unwrap_or(1.0).to_string())
        .arg(preset_secs.unwrap_or(30.0).to_string())
        .arg(preset.unwrap_or_default())
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| IpcError::new("spawn", format!("couldn't start visualizer: {e}")))?;

    let raw_stdin = child
        .stdin
        .take()
        .ok_or_else(|| IpcError::new("spawn", "no stdin pipe to the visualizer"))?;
    let stdin = Arc::new(Mutex::new(raw_stdin));

    let tap = engine.spectrum();
    let playing = engine.playing_flag();
    let stop = Arc::new(AtomicBool::new(false));
    let run = stop.clone();
    let pump_stdin = stdin.clone();
    let pump = std::thread::Builder::new()
        .name("hm-viz-pcm".into())
        .spawn(move || {
            let period = Duration::from_millis(1000 / PCM_FPS);
            let mut buf = Vec::with_capacity(1 + 2048);
            while !run.load(Ordering::Relaxed) {
                // Nothing playing → the waveform is just silence; don't stream
                // 2 KB frames at 30 Hz to an idle window. Poll gently and resume
                // full-rate the moment playback starts (the same flag the
                // engine's UI-forwarding thread watches).
                if !playing.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                buf.clear();
                buf.push(b'P'); // PCM frame tag
                for s in tap.load_waveform() {
                    buf.extend_from_slice(&s.to_le_bytes());
                }
                // The window closing breaks the pipe — stop quietly.
                let broken = pump_stdin
                    .lock()
                    .map(|mut s| s.write_all(&buf).is_err())
                    .unwrap_or(true);
                if broken {
                    break;
                }
                std::thread::sleep(period);
            }
        })
        .ok();

    *state.inner.lock().expect("visualizer poisoned") = Some(Running {
        child,
        stop,
        pump,
        stdin,
    });
    Ok(())
}

/// Switch the open visualizer window to `preset` (a `.milk` file stem). No-op
/// when the window isn't open.
#[tauri::command]
pub fn visualizer_set_preset(state: State<'_, VisualizerState>, preset: String) {
    // Clone the shared pipe out and drop the state lock before writing.
    let stdin = {
        let guard = state.inner.lock().expect("visualizer poisoned");
        match guard.as_ref() {
            Some(running) => running.stdin.clone(),
            None => return,
        }
    };
    let bytes = preset.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    let mut msg = Vec::with_capacity(3 + len);
    msg.push(b'L'); // load-preset tag
    msg.extend_from_slice(&(len as u16).to_le_bytes());
    msg.extend_from_slice(&bytes[..len]);
    let _ = stdin.lock().map(|mut s| s.write_all(&msg));
}

/// Close the visualizer window and stop streaming.
#[tauri::command]
pub fn visualizer_stop(state: State<'_, VisualizerState>) {
    if let Some(r) = state.inner.lock().expect("visualizer poisoned").take() {
        r.shutdown();
    }
}

/// Whether the visualizer window is currently open.
#[tauri::command]
pub fn visualizer_is_open(state: State<'_, VisualizerState>) -> bool {
    state.inner.lock().expect("visualizer poisoned").is_some()
}
