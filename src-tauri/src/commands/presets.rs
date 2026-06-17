//! EQ preset commands (SQLite-backed) and preset application.

use hm_audio::AudioEngine;
use hm_core::{EqPreset, IpcError, PresetStore, BAND_COUNT};
use tauri::State;

fn to_band_array(bands: Vec<f32>) -> Result<[f32; BAND_COUNT], IpcError> {
    bands
        .try_into()
        .map_err(|_| IpcError::new("invalid", "expected 31 EQ bands"))
}

/// All presets (built-in genre presets first, then custom).
#[tauri::command]
pub fn eq_list_presets(store: State<'_, PresetStore>) -> Result<Vec<EqPreset>, IpcError> {
    store.list().map_err(Into::into)
}

/// Apply a preset to the engine and mark it active. Returns the applied preset.
#[tauri::command]
pub fn eq_apply_preset(
    store: State<'_, PresetStore>,
    engine: State<'_, AudioEngine>,
    id: String,
) -> Result<EqPreset, IpcError> {
    let preset = store.get(&id)?;
    engine.apply_eq_preset(preset.bands, preset.pre_gain, preset.id.clone());
    Ok(preset)
}

/// Save the current band curve as a new custom preset.
#[tauri::command]
pub fn eq_save_custom(
    store: State<'_, PresetStore>,
    name: String,
    bands: Vec<f32>,
    pre_gain: f32,
) -> Result<EqPreset, IpcError> {
    let bands = to_band_array(bands)?;
    store
        .save_custom(&name, bands, pre_gain)
        .map_err(Into::into)
}

/// Update an existing custom preset.
#[tauri::command]
pub fn eq_update(store: State<'_, PresetStore>, preset: EqPreset) -> Result<(), IpcError> {
    store.update(&preset).map_err(Into::into)
}

/// Delete a custom preset.
#[tauri::command]
pub fn eq_delete(store: State<'_, PresetStore>, id: String) -> Result<(), IpcError> {
    store.delete(&id).map_err(Into::into)
}
