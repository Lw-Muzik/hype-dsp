//! Audio device commands.

use hm_audio::DeviceInfo;
use hm_core::IpcError;

/// List the system's output (playback) devices.
///
/// Real data from the platform audio backend — the first non-trivial command
/// returning a typed list to the UI. Device selection and streaming arrive with
/// the engine in Phase 2.
// `(async)`: cpal device enumeration can be slow (esp. WASAPI on Windows) — run
// it off the Tauri main thread so opening Settings / startup doesn't stall.
#[tauri::command(async)]
pub fn audio_list_output_devices() -> Result<Vec<DeviceInfo>, IpcError> {
    Ok(hm_audio::list_output_devices()?)
}

/// List the system's input (capture) devices.
#[tauri::command(async)]
pub fn audio_list_input_devices() -> Result<Vec<DeviceInfo>, IpcError> {
    Ok(hm_audio::list_input_devices()?)
}
