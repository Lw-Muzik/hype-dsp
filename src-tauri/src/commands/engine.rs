//! Engine and playback commands.
//!
//! Parameter setters write into the lock-free engine snapshot; playback
//! commands message the engine's control thread. Real-time meter frames are not
//! polled here — they are pushed to the UI over the `engine:frame` event by the
//! forwarder thread (see `lib.rs`).

use std::path::PathBuf;

use hm_audio::AudioEngine;
use hm_core::{EngineState, IpcError};
use tauri::State;

/// Current engine state (mirrored by the Zustand store on startup).
#[tauri::command]
pub fn engine_get_state(engine: State<'_, AudioEngine>) -> EngineState {
    engine.state()
}

/// Toggle the global enhancement power (chain bypass).
#[tauri::command]
pub fn engine_set_power(engine: State<'_, AudioEngine>, power: bool) {
    engine.set_power(power);
}

/// Set the master output volume (linear gain).
#[tauri::command]
pub fn engine_set_master_volume(engine: State<'_, AudioEngine>, volume: f32) {
    engine.set_master_volume(volume);
}

/// Decode and play a local file through the enhancement chain.
#[tauri::command]
pub fn player_play_file(engine: State<'_, AudioEngine>, path: String) -> Result<(), IpcError> {
    engine.play_file(&PathBuf::from(path)).map_err(Into::into)
}

/// Stop playback.
#[tauri::command]
pub fn player_stop(engine: State<'_, AudioEngine>) {
    engine.stop();
}

/// Whether audio is currently playing.
#[tauri::command]
pub fn player_is_playing(engine: State<'_, AudioEngine>) -> bool {
    engine.is_playing()
}
