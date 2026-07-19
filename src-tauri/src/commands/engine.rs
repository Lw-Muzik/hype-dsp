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

/// Parse an EqualizerAPO GraphicEQ string, map it onto the 31 ISO bands with a
/// clip-proof preamp, apply it to the engine, and return the resolved values.
///
/// Shared by `engine_eq_import_graphic` (pasted curve) and the AutoEQ fetch
/// command (curve fetched from the bundled index's URL) so both consume the
/// exact same import pipeline.
pub fn apply_graphic_curve(
    engine: &AudioEngine,
    curve: &str,
) -> Result<EqImportResult, IpcError> {
    let points = hm_core::parse_graphic_eq(curve).map_err(|e| IpcError::new("invalid", &e))?;
    let bands = hm_core::interpolate_to_iso_bands(&points);
    let pre_gain = hm_core::recommended_preamp(&bands);
    engine.set_eq(bands, pre_gain, true);
    Ok(EqImportResult { bands: bands.to_vec(), pre_gain })
}

/// Parse an EqualizerAPO GraphicEQ string, map it onto the 31 bands with a
/// clip-proof preamp, apply it, and return the resolved values to the UI.
#[tauri::command]
pub fn engine_eq_import_graphic(
    engine: State<'_, AudioEngine>,
    curve: String,
) -> Result<EqImportResult, IpcError> {
    apply_graphic_curve(&engine, &curve)
}

/// Import a ViPER4Android / JamesDSP DDC (`.vdc`) file: read it, evaluate its
/// biquad cascade's magnitude response onto the 31 ISO bands with a clip-proof
/// preamp, apply it, and return the resolved values to the UI.
#[tauri::command]
pub fn engine_eq_import_vdc(
    engine: State<'_, AudioEngine>,
    path: String,
) -> Result<EqImportResult, IpcError> {
    let body = std::fs::read_to_string(&path)
        .map_err(|e| IpcError::new("io", format!("couldn't read {path}: {e}")))?;
    let bands = hm_core::vdc_to_iso_bands(&body).map_err(|e| IpcError::new("invalid", e))?;
    let pre_gain = hm_core::recommended_preamp(&bands);
    engine.set_eq(bands, pre_gain, true);
    Ok(EqImportResult { bands: bands.to_vec(), pre_gain })
}

/// Names of all bundled ViPER DDC presets (sorted) for the EQ library browser.
#[tauri::command]
pub fn ddc_list() -> Vec<String> {
    hm_core::ddc_library::names()
}

/// Apply a bundled ViPER DDC preset by name to the 31-band EQ (same resolution
/// as importing a `.vdc`, but the content is shipped with the app).
#[tauri::command]
pub fn engine_eq_apply_ddc(
    engine: State<'_, AudioEngine>,
    name: String,
) -> Result<EqImportResult, IpcError> {
    let body = hm_core::ddc_library::get(&name)
        .ok_or_else(|| IpcError::new("not_found", format!("DDC preset '{name}' not found")))?;
    let bands = hm_core::vdc_to_iso_bands(body).map_err(|e| IpcError::new("invalid", e))?;
    let pre_gain = hm_core::recommended_preamp(&bands);
    engine.set_eq(bands, pre_gain, true);
    Ok(EqImportResult {
        bands: bands.to_vec(),
        pre_gain,
    })
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

/// Compile a LiveProg (EEL2-subset) script and load it into the DSP chain.
///
/// `async` deliberately. Compiling is cheap — lex, parse and emit over a few
/// hundred characters — but a sync command runs on the main thread, and this app
/// has twice shipped one that froze the UI (lyrics, mixer icon extraction). The
/// cost of not repeating that here is a keyword.
///
/// On success the program is published lock-free to the script stage and the
/// source is stored in engine state, so it survives a restart and is captured by
/// whole-chain presets. On failure nothing is published, the previously-running
/// script keeps playing, and the caller gets the line and column.
#[tauri::command]
pub async fn engine_script_compile(
    engine: State<'_, AudioEngine>,
    source: String,
) -> Result<(), IpcError> {
    engine
        .compile_script(source)
        .map_err(|e| IpcError::new("script", format!("[{}:{}] {}", e.line, e.col, e.message)))
}

/// Enable or disable the LiveProg script stage without recompiling it.
#[tauri::command]
pub fn engine_set_script(engine: State<'_, AudioEngine>, enabled: bool) {
    engine.set_script(enabled);
}

/// Configure the output stage — notably the brickwall limiter on/off. The
/// limiter is on by default; turning it off removes the clipping safety net.
#[tauri::command]
pub fn engine_set_output(engine: State<'_, AudioEngine>, output: hm_core::OutputState) {
    engine.set_output(output);
}

/// Load an impulse-response file into the convolver (heavy prep off the audio thread).
// `(async)`: loading + FFT-partitioning an impulse response does file I/O and
// heavy CPU — run it off the Tauri main thread so the UI doesn't stall.
#[tauri::command(async)]
pub fn engine_convolver_load_ir(
    engine: State<'_, AudioEngine>,
    path: String,
) -> Result<hm_audio::ConvolverIrInfo, IpcError> {
    engine
        .load_convolver_ir(&PathBuf::from(path))
        .map_err(Into::into)
}

/// Decode and play a local file through the enhancement chain.
// `(async)`: `play_file` decodes the whole local file before handing it to the
// engine — run it off the Tauri main thread so playing a large FLAC/WAV doesn't
// freeze the UI.
#[tauri::command(async)]
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

/// Toggle Data Saver (low-bandwidth) mode.
#[tauri::command]
pub fn engine_set_data_saver(engine: State<'_, AudioEngine>, on: bool) {
    engine.set_data_saver(on);
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
// `(async)`: enumerating input devices can take seconds when Bluetooth audio is
// involved, and opening the capture stream isn't instant either — keep both off
// the Tauri main thread.
#[tauri::command(async)]
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
// `(async)`: building the Core Audio process tap + aggregate device (macOS) or
// the re-routing pipeline (Linux/Windows) is a known-slow op — run it off the
// Tauri main thread so the UI doesn't freeze while it's set up.
#[tauri::command(async)]
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

/// Set which apps the system-wide EQ tap processes (macOS). The change is stored
/// in `EngineState.system_eq_scope`; the caller re-invokes `player_play_system_audio`
/// to rebuild the tap if system-wide EQ is currently on.
#[tauri::command]
pub fn system_eq_set_scope(engine: State<'_, AudioEngine>, scope: hm_core::SystemEqScope) {
    engine.set_system_eq_scope(scope);
}

/// Stop system-wide equalization and restore normal audio routing. On macOS this
/// stops playback; on Linux/Windows it tears down the re-routing pipeline.
// `(async)`: teardown joins the routing worker / rebuilds audio routing
// synchronously — run it off the Tauri main thread like its `player_play_*`
// counterpart.
#[tauri::command(async)]
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

/// Per-OS readiness of system-wide equalization, so the Settings card can show the
/// right affordance: enable it, or (Windows) install the bundled audio driver first.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioStatus {
    /// The OS supports system-wide EQ at all (show the card).
    pub supported: bool,
    /// Ready to enable right now (macOS: tap available; Linux: PulseAudio/PipeWire
    /// present; Windows: the bundled virtual-audio driver is installed).
    pub available: bool,
    /// Windows-only: the bundled virtual-audio driver is installed (always `true`
    /// on macOS/Linux, which need no driver).
    pub driver_installed: bool,
    /// This OS routes through a bundled driver the user may need to install
    /// (Windows only) — gates the in-app "Install audio driver" action.
    pub needs_driver: bool,
}

/// Report whether system-wide EQ is supported, ready, and (Windows) whether the
/// bundled audio driver is installed — one round-trip for the Settings card.
// `(async)`: the Windows probe enumerates audio endpoints over COM/WASAPI (and
// macOS asks Core Audio) — not main-thread work.
#[tauri::command(async)]
pub fn system_audio_status() -> SystemAudioStatus {
    #[cfg(target_os = "macos")]
    {
        SystemAudioStatus {
            supported: true,
            available: hm_audio::system_tap::available(),
            driver_installed: true,
            needs_driver: false,
        }
    }
    #[cfg(target_os = "linux")]
    {
        let available = hm_audio::system_eq_available();
        SystemAudioStatus {
            supported: true,
            available,
            driver_installed: true,
            needs_driver: false,
        }
    }
    #[cfg(target_os = "windows")]
    {
        let installed = hm_audio::win_driver::routing_device_available();
        SystemAudioStatus {
            supported: true,
            available: installed,
            driver_installed: installed,
            needs_driver: true,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        SystemAudioStatus {
            supported: false,
            available: false,
            driver_installed: false,
            needs_driver: false,
        }
    }
}

/// Current runtime state of system-wide EQ: `"active"`, `"recovering"`, or
/// `"disabled"`. Poll this to reflect recovery in the UI — notably on macOS, where
/// a tap stall under heavy load is now recovered in the background (the card can
/// show "recovering…") instead of the EQ appearing to silently stop.
#[tauri::command]
pub fn system_eq_status(engine: State<'_, AudioEngine>) -> hm_audio::SystemEqStatus {
    engine.system_eq_status()
}

/// Install the bundled Windows virtual-audio driver (prompts for admin via UAC).
/// No-op success on platforms that need no driver. The frontend should re-query
/// [`system_audio_status`] afterwards to confirm the device enumerated.
// `(async)`: runs `pnputil` under a UAC prompt — run it off the Tauri main
// thread so the whole webview isn't frozen for the elevation + install.
#[tauri::command(async)]
pub fn system_audio_install_driver(app: tauri::AppHandle) -> Result<(), IpcError> {
    #[cfg(target_os = "windows")]
    {
        use tauri::Manager;
        let dir = app.path().resource_dir().map_err(|e| {
            IpcError::new("driver", format!("could not resolve app resources: {e}"))
        })?;
        let package_dir = dir.join("drivers").join("HypeMuzikAudio");
        hm_audio::win_driver::install_driver(&package_dir).map_err(Into::into)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Ok(())
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
