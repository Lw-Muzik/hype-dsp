//! Engine and playback commands.
//!
//! Parameter setters write into the lock-free engine snapshot; playback
//! commands message the engine's control thread. Real-time meter frames are not
//! polled here — they are pushed to the UI over the `engine:frame` event by the
//! forwarder thread (see `lib.rs`).

use std::path::PathBuf;

use hm_audio::AudioEngine;
use hm_core::{EngineState, IpcError, SpatialMode};
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

/// Apply a manual 31-band EQ edit (clears the active preset).
#[tauri::command]
pub fn engine_set_eq(
    engine: State<'_, AudioEngine>,
    bands: Vec<f32>,
    pre_gain: f32,
    enabled: bool,
) -> Result<(), IpcError> {
    let bands: [f32; hm_core::BAND_COUNT] = bands
        .try_into()
        .map_err(|_| IpcError::new("invalid", "expected 31 EQ bands"))?;
    engine.set_eq(bands, pre_gain, enabled);
    Ok(())
}

/// Configure the bass boost stage.
#[tauri::command]
pub fn engine_set_bass(
    engine: State<'_, AudioEngine>,
    enabled: bool,
    amount: f32,
    harmonics: bool,
) {
    engine.set_bass(enabled, amount, harmonics);
}

/// Configure the spatializer (surround) stage.
#[tauri::command]
pub fn engine_set_spatializer(
    engine: State<'_, AudioEngine>,
    enabled: bool,
    amount: f32,
    mode: SpatialMode,
) {
    engine.set_spatializer(enabled, amount, mode);
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

/// Pause playback (keeps position).
#[tauri::command]
pub fn player_pause(engine: State<'_, AudioEngine>) {
    engine.pause();
}

/// Resume playback.
#[tauri::command]
pub fn player_resume(engine: State<'_, AudioEngine>) {
    engine.resume();
}

/// Seek to `secs` within the current track.
#[tauri::command]
pub fn player_seek(engine: State<'_, AudioEngine>, secs: f64) {
    engine.seek(secs);
}

/// Whether audio is currently playing.
#[tauri::command]
pub fn player_is_playing(engine: State<'_, AudioEngine>) -> bool {
    engine.is_playing()
}
