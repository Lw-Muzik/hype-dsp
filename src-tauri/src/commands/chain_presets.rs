//! Whole-chain DSP preset commands (JSON-file-backed).
//!
//! These commands let the UI save, recall, delete, export and import complete
//! [`EngineState`] snapshots under human-readable names.  Unlike the EQ-only
//! [`PresetStore`], these capture every stage of the enhancement chain at once.
//!
//! The store has no internal lock so it is wrapped in a [`Mutex`] when managed
//! and locked in each command body.

use std::sync::Mutex;

use hm_audio::AudioEngine;
use hm_core::{ChainPreset, ChainPresetStore, IpcError};
use tauri::State;

/// List all saved whole-chain presets.
#[tauri::command]
pub fn chain_preset_list(
    store: State<'_, Mutex<ChainPresetStore>>,
) -> Result<Vec<ChainPreset>, IpcError> {
    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    store.list().map_err(Into::into)
}

/// Save the current engine state as a new whole-chain preset named `name`.
///
/// Returns the newly created preset (with its generated id) so the UI can
/// append it to the list without a round-trip.
#[tauri::command]
pub fn chain_preset_save(
    engine: State<'_, AudioEngine>,
    store: State<'_, Mutex<ChainPresetStore>>,
    name: String,
) -> Result<ChainPreset, IpcError> {
    let current = engine.state();
    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    store.save(&name, current).map_err(Into::into)
}

/// Apply a saved preset to the engine, preserving the current `power`,
/// `master_volume`, and `system_eq_scope` settings.
///
/// A preset captures a *sound*, not the user's output level, bypass switch, or
/// which apps the system-EQ tap covers, so those fields are taken from the
/// running engine rather than the preset.
#[tauri::command]
pub fn chain_preset_apply(
    engine: State<'_, AudioEngine>,
    store: State<'_, Mutex<ChainPresetStore>>,
    id: String,
) -> Result<(), IpcError> {
    // Read the current state BEFORE locking the store so we hold the lock for
    // the shortest possible time.
    let current = engine.state();

    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    let presets = store.list().map_err(|e: hm_core::HmError| IpcError::from(e))?;
    let preset = presets
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IpcError::new("not_found", format!("chain preset {id} not found")))?;

    // Build the applied state from the preset but keep power + master_volume
    // from the current engine so the user's volume/bypass is undisturbed.
    let mut applied = preset.state;
    applied.power = current.power;
    applied.master_volume = current.master_volume;
    applied.system_eq_scope = current.system_eq_scope;

    drop(store); // release the lock before touching the engine
    engine.set_state(applied);
    Ok(())
}

/// Delete a saved whole-chain preset by id.
#[tauri::command]
pub fn chain_preset_delete(
    store: State<'_, Mutex<ChainPresetStore>>,
    id: String,
) -> Result<(), IpcError> {
    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    store.delete(&id).map_err(Into::into)
}

/// Export a preset to a JSON file at `path`.
///
/// The file is a pretty-printed [`ChainPreset`] that can be imported on any
/// machine with [`chain_preset_import`].
#[tauri::command]
pub fn chain_preset_export(
    store: State<'_, Mutex<ChainPresetStore>>,
    id: String,
    path: String,
) -> Result<(), IpcError> {
    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    let presets = store.list().map_err(|e: hm_core::HmError| IpcError::from(e))?;
    let preset = presets
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IpcError::new("not_found", format!("chain preset {id} not found")))?;

    let json = serde_json::to_string_pretty(&preset)
        .map_err(|e| IpcError::new("serde", e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| IpcError::new("io", e.to_string()))?;
    Ok(())
}

/// Import a preset from a JSON file at `path`.
///
/// The incoming id is replaced with a fresh one to avoid collisions with
/// locally saved presets.  Returns the stored preset.
#[tauri::command]
pub fn chain_preset_import(
    store: State<'_, Mutex<ChainPresetStore>>,
    path: String,
) -> Result<ChainPreset, IpcError> {
    let text =
        std::fs::read_to_string(&path).map_err(|e| IpcError::new("io", e.to_string()))?;
    let preset: ChainPreset = serde_json::from_str(&text)
        .map_err(|e| IpcError::new("serde", format!("invalid preset file: {e}")))?;

    let store = store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?;
    store.upsert_imported(preset).map_err(Into::into)
}
