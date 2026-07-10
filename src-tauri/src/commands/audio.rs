//! Audio device commands.

use hm_audio::{DeviceInfo, OutputDevice};
use hm_core::IpcError;

/// List the system's output devices for the picker, with a stable UID, the
/// current-default flag, and a transport type for the UI icon.
///
/// macOS reads Core Audio directly (UID + transport); other platforms fall back
/// to a names-only cpal listing. `(async)`: Core Audio / cpal enumeration can be
/// slow, so it runs off the Tauri main thread (the same reason the mixer session
/// list is async) — opening Settings never stalls the UI.
#[tauri::command(async)]
pub fn audio_output_devices() -> Result<Vec<OutputDevice>, IpcError> {
    Ok(hm_audio::output_device::list_output_devices()?)
}

/// Make the given device (by UID) the **system default** output device.
///
/// The engine follows the system default, so this moves all of the app's audio
/// (and the whole system's) to the chosen device. Needs no special entitlement.
/// Returns a human-readable message on failure (unknown UID, the set failing, or
/// the change not sticking) so the UI can surface it in a toast.
#[tauri::command(async)]
pub fn audio_set_default_output(uid: String) -> Result<(), String> {
    hm_audio::output_device::set_default_output(&uid).map_err(|e| e.to_string())
}

/// List the system's input (capture) devices.
#[tauri::command(async)]
pub fn audio_list_input_devices() -> Result<Vec<DeviceInfo>, IpcError> {
    Ok(hm_audio::list_input_devices()?)
}
