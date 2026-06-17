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

#[tauri::command]
pub fn mixer_list_sessions(mixer: State<'_, Mixer>) -> MixerSnapshot {
    let ctrl = mixer.lock().expect("mixer poisoned");
    MixerSnapshot {
        supported: ctrl.supported(),
        unavailable_reason: ctrl.unavailable_reason(),
        sessions: ctrl.list_sessions(),
    }
}

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
