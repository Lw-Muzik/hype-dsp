//! Headphone profile commands (bundled AutoEq dataset) + application.

use hm_audio::AudioEngine;
use hm_core::{headphones, HeadphoneProfile, IpcError};
use tauri::State;

/// All bundled headphone correction profiles.
#[tauri::command]
pub fn profile_list() -> Vec<HeadphoneProfile> {
    headphones::bundled()
}

/// Load a profile's correction into the chain and mark it active.
#[tauri::command]
pub fn profile_set_active(
    engine: State<'_, AudioEngine>,
    id: String,
) -> Result<HeadphoneProfile, IpcError> {
    let profile = headphones::get(&id)
        .ok_or_else(|| IpcError::new("not_found", format!("headphone profile {id}")))?;
    engine.set_headphone(profile.bands.clone(), profile.preamp, profile.id.clone());
    Ok(profile)
}

/// Clear any active headphone correction.
#[tauri::command]
pub fn profile_clear(engine: State<'_, AudioEngine>) {
    engine.clear_headphone();
}
