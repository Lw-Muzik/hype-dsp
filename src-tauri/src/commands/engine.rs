//! Engine and playback commands.
//!
//! Parameter setters write into the lock-free engine snapshot; playback
//! commands message the engine's control thread. Real-time meter frames are not
//! polled here — they are pushed to the UI over the `engine:frame` event by the
//! forwarder thread (see `lib.rs`).

use std::path::PathBuf;

use hm_audio::AudioEngine;
use hm_core::{EngineState, IpcError, RoomState, SpatialMode, SurroundSpeakers};
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

/// Result of importing a GraphicEQ curve: the resolved bands + clip-proof preamp.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqImportResult {
    pub bands: Vec<f32>,
    pub pre_gain: f32,
}

/// Parse an EqualizerAPO GraphicEQ string, map it onto the 31 bands with a
/// clip-proof preamp, apply it, and return the resolved values to the UI.
#[tauri::command]
pub fn engine_eq_import_graphic(
    engine: State<'_, AudioEngine>,
    curve: String,
) -> Result<EqImportResult, IpcError> {
    let points = hm_core::parse_graphic_eq(&curve)
        .map_err(|e| IpcError::new("invalid", &e))?;
    let bands = hm_core::interpolate_to_iso_bands(&points);
    let pre_gain = hm_core::recommended_preamp(&bands);
    engine.set_eq(bands, pre_gain, true);
    Ok(EqImportResult { bands: bands.to_vec(), pre_gain })
}

/// Configure the bass boost stage.
#[tauri::command]
pub fn engine_set_bass(
    engine: State<'_, AudioEngine>,
    enabled: bool,
    amount: f32,
    harmonics: bool,
    adaptive: bool,
) {
    engine.set_bass(enabled, amount, harmonics, adaptive);
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

/// Configure the 3D-surround (virtual-speaker) stage.
#[tauri::command]
pub fn engine_set_surround3d(
    engine: State<'_, AudioEngine>,
    enabled: bool,
    intensity: f32,
    subwoofer: f32,
    speakers: SurroundSpeakers,
) {
    engine.set_surround3d(enabled, intensity, subwoofer, speakers);
}

/// Configure the room-reverb ("room effects") stage.
#[tauri::command]
pub fn engine_set_room(engine: State<'_, AudioEngine>, room: RoomState) {
    engine.set_room(room);
}

/// Configure the convolver stage's scalar params.
#[tauri::command]
pub fn engine_set_convolver(engine: State<'_, AudioEngine>, convolver: hm_core::ConvolverState) {
    engine.set_convolver(convolver);
}

/// Configure the multiband compander stage.
#[tauri::command]
pub fn engine_set_compander(engine: State<'_, AudioEngine>, compander: hm_core::CompanderState) {
    engine.set_compander(compander);
}

/// Configure the tube saturation stage.
#[tauri::command]
pub fn engine_set_saturation(engine: State<'_, AudioEngine>, saturation: hm_core::SaturationState) {
    engine.set_saturation(saturation);
}

/// Load an impulse-response file into the convolver (heavy prep off the audio thread).
#[tauri::command]
pub fn engine_convolver_load_ir(
    engine: State<'_, AudioEngine>,
    path: String,
) -> Result<hm_audio::ConvolverIrInfo, IpcError> {
    engine
        .load_convolver_ir(&PathBuf::from(path))
        .map_err(Into::into)
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

/// Update gapless + crossfade playback behaviour.
#[tauri::command]
pub fn engine_set_playback(engine: State<'_, AudioEngine>, gapless: bool, crossfade_secs: f32) {
    engine.set_playback(gapless, crossfade_secs);
}

/// Play a list of local files as a gapless (and optionally crossfading) queue,
/// starting at `start`. The crossfade duration is read live from the engine's
/// playback settings each block, so changing it applies to the current queue.
#[tauri::command]
pub fn player_play_queue(
    engine: State<'_, AudioEngine>,
    paths: Vec<String>,
    start: usize,
) -> Result<(), IpcError> {
    engine.play_queue(paths, start).map_err(Into::into)
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

/// Whether system-wide equalization is available on this machine: macOS uses
/// Core Audio process taps (14.4+, permission requested on first use); Linux a
/// PulseAudio/PipeWire virtual sink; Windows the bundled virtual audio device.
#[tauri::command]
pub fn system_audio_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        hm_audio::system_tap::available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        hm_audio::system_eq_available()
    }
}

/// Equalize system-wide audio through the chain. macOS taps every other app and
/// re-renders the processed mix; Linux/Windows re-route all output through a
/// virtual device into the chain. Returns a clear error if unavailable/denied.
#[tauri::command]
pub fn player_play_system_audio(engine: State<'_, AudioEngine>) -> Result<(), IpcError> {
    #[cfg(target_os = "macos")]
    {
        engine.play_system_tap().map_err(Into::into)
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        engine.start_system_eq().map_err(Into::into)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = engine;
        Err(IpcError::new(
            "unsupported",
            "System-wide equalization isn't supported on this platform.",
        ))
    }
}

/// Stop system-wide equalization and restore normal audio routing. On macOS this
/// stops playback; on Linux/Windows it tears down the re-routing pipeline.
#[tauri::command]
pub fn stop_system_audio(engine: State<'_, AudioEngine>) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        engine.stop_system_eq();
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        engine.stop();
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
