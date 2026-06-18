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

/// Stream and play an internet radio URL through the chain.
#[tauri::command]
pub fn player_play_radio(engine: State<'_, AudioEngine>, url: String) -> Result<(), IpcError> {
    engine.play_radio(url).map_err(Into::into)
}

/// Capture the default input device through the chain (driver-free stand-in).
#[tauri::command]
pub fn player_play_capture(engine: State<'_, AudioEngine>) -> Result<(), IpcError> {
    if hm_audio::list_input_devices()
        .map(|d| d.is_empty())
        .unwrap_or(true)
    {
        return Err(IpcError::new(
            "unavailable",
            "No audio input device available.",
        ));
    }
    engine.play_capture().map_err(Into::into)
}

/// Whether true system-wide capture (a signed virtual device) is installed.
#[tauri::command]
pub fn capture_virtual_available() -> bool {
    hm_audio::virtual_device_available()
}

/// Whether system-wide capture via Core Audio process taps is available
/// (macOS 14.4+). The audio-capture permission is requested on first use.
#[tauri::command]
pub fn system_audio_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        hm_audio::system_tap::available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Equalize system-wide audio through the chain (macOS process tap). Returns a
/// clear error if denied/unavailable.
#[tauri::command]
pub fn player_play_system_audio(engine: State<'_, AudioEngine>) -> Result<(), IpcError> {
    #[cfg(target_os = "macos")]
    {
        engine.play_system_tap().map_err(Into::into)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = engine;
        Err(IpcError::new(
            "unsupported",
            "System-wide capture via process taps is macOS-only.",
        ))
    }
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
