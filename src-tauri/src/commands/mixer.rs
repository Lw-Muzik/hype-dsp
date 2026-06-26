//! Per-application mixer commands.

use std::sync::Mutex;

use hm_core::{AppSession, IpcError};
use hm_platform::SessionController;
use serde::Serialize;
use tauri::State;

/// Managed state: the platform session controller behind a mutex (COM access
/// on Windows must be serialized).
pub type Mixer = Mutex<Box<dyn SessionController>>;

/// Mixer snapshot for the UI, including the unsupported state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerSnapshot {
    pub supported: bool,
    pub unavailable_reason: Option<String>,
    pub sessions: Vec<AppSession>,
}

/// MUST stay `async`: enumeration resolves each app's identity and icon
/// (`NSRunningApplication` → `NSImage` → TIFF → PNG → base64 on macOS), which is
/// expensive on first open. A plain `#[tauri::command]` runs on Tauri's main
/// thread — the same thread driving the webview — so it would freeze the UI
/// until enumeration finished. `(async)` runs it on a worker thread instead.
#[tauri::command(async)]
pub fn mixer_list_sessions(mixer: State<'_, Mixer>) -> MixerSnapshot {
    let ctrl = mixer.lock().expect("mixer poisoned");
    MixerSnapshot {
        supported: ctrl.supported(),
        unavailable_reason: ctrl.unavailable_reason(),
        sessions: ctrl.list_sessions(),
    }
}

// `mixer_set_volume`/`mixer_set_muted` stay synchronous on purpose: a dragged
// slider fires a burst of these, and main-thread commands are applied in arrival
// order, so the last value the user picked wins. They are cheap (a desired-state
// write plus, at most, one tap creation), so they do not freeze the UI like
// enumeration did. Do NOT make them `(async)` — concurrent execution could apply
// an earlier value last and leave the app at the wrong volume.
#[tauri::command]
pub fn mixer_set_volume(mixer: State<'_, Mixer>, id: String, gain: f32) -> Result<(), IpcError> {
    mixer
        .lock()
        .expect("mixer poisoned")
        .set_volume(&id, gain)
        .map_err(Into::into)
}

#[tauri::command]
pub fn mixer_set_muted(mixer: State<'_, Mixer>, id: String, muted: bool) -> Result<(), IpcError> {
    mixer
        .lock()
        .expect("mixer poisoned")
        .set_muted(&id, muted)
        .map_err(Into::into)
}
