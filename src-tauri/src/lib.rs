//! HypeMuzik desktop — Tauri application entry point.
//!
//! Wires the internal crates (`hm-*`) to the webview UI: creates the audio
//! engine, registers plugins and the typed command handlers, spawns the
//! meter-frame forwarder, then runs the event loop. The heavy lifting (DSP,
//! audio engine, media, persistence) lives in the workspace crates; this layer
//! is the thin, well-documented bridge between Rust and React.

mod commands;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use std::sync::Mutex;

use hm_audio::{AudioEngine, EngineMeters, PlaybackPos, SpectrumTap, SPECTRUM_BANDS};
use hm_core::{EngineFrame, EngineState, LicenseMock, MediaStore, MeterFrame, PresetStore};
use serde::Serialize;
use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

/// Transport progress payload (`engine:progress`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    position_secs: f64,
    duration_secs: Option<f64>,
    paused: bool,
}

/// Emits real-time meter + spectrum frames to the UI at ~60 fps over the
/// `engine:frame` event, and play/stop transitions over `engine:transport`.
/// Runs for the app's lifetime on its own thread; it only reads lock-free
/// telemetry.
fn forward_frames(
    app: tauri::AppHandle,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) {
    let mut last_playing = false;
    let mut tick: u32 = 0;
    loop {
        std::thread::sleep(Duration::from_millis(16));
        tick = tick.wrapping_add(1);
        let now_playing = playing.load(Ordering::Relaxed);

        if now_playing != last_playing {
            let _ = app.emit("engine:transport", now_playing);
            if !now_playing {
                // Settle meters and spectrum to idle when playback ends.
                let _ = app.emit(
                    "engine:frame",
                    EngineFrame {
                        meters: MeterFrame::default(),
                        spectrum: Some(vec![0.0; SPECTRUM_BANDS]),
                    },
                );
            }
            last_playing = now_playing;
        }

        if now_playing {
            let _ = app.emit(
                "engine:frame",
                EngineFrame {
                    meters: meters.load(),
                    spectrum: Some(spectrum.load()),
                },
            );
            // Transport progress at ~10 fps (every ~6 ticks).
            if tick % 6 == 0 {
                let _ = app.emit(
                    "engine:progress",
                    Progress {
                        position_secs: pos.position_secs(),
                        duration_secs: pos.duration_secs(),
                        paused: paused.load(Ordering::Relaxed),
                    },
                );
            }
        }
    }
}

/// Build and run the Tauri application.
pub fn run() {
    let engine = AudioEngine::new();
    let meters = engine.meters();
    let spectrum = engine.spectrum();
    let pos = engine.pos();
    let playing = engine.playing_flag();
    let paused = engine.paused_flag();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        // Replace the default File/Edit/View/Window/Help menu with just the
        // standard app menu (About/Quit), so ⌘Q still works on macOS.
        .menu(|handle| {
            let app_menu = Submenu::with_items(
                handle,
                "HypeMuzik",
                true,
                &[
                    &PredefinedMenuItem::about(handle, None, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::hide(handle, None)?,
                    &PredefinedMenuItem::quit(handle, None)?,
                ],
            )?;
            Menu::with_items(handle, &[&app_menu])
        })
        .manage(engine)
        .setup(move |app| {
            // Open the preset store in the app data dir; fall back to an
            // in-memory store so the app still runs if the disk path fails.
            let store = app
                .path()
                .app_data_dir()
                .ok()
                .and_then(|dir| {
                    std::fs::create_dir_all(&dir).ok()?;
                    PresetStore::open(&dir.join("hypemuzik.db")).ok()
                })
                .or_else(|| PresetStore::open_in_memory().ok());
            if let Some(store) = store {
                app.manage(store);
            }

            // Library + playlists store (separate DB file).
            let media = app
                .path()
                .app_data_dir()
                .ok()
                .and_then(|dir| {
                    std::fs::create_dir_all(&dir).ok()?;
                    MediaStore::open(&dir.join("library.db")).ok()
                })
                .or_else(|| MediaStore::open_in_memory().ok());
            if let Some(media) = media {
                app.manage(media);
            }

            // License mock (persists trial/key to disk).
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                app.manage(LicenseMock::open(dir.join("license.json")));
            } else {
                app.manage(LicenseMock::open(
                    std::env::temp_dir().join("hm_license.json"),
                ));
            }

            // Per-app mixer controller (real on Windows; unsupported stub on macOS).
            app.manage::<commands::mixer::Mixer>(Mutex::new(hm_platform::default_controller()));

            // Restore the user's saved settings (EQ, bass, surround, volume, …)
            // from disk, then autosave them whenever they change so the next
            // launch comes up exactly as they left it.
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join("engine-state.json");
                let engine = app.state::<AudioEngine>();
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(state) = serde_json::from_str::<EngineState>(&text) {
                        engine.set_state(state);
                    }
                }
                let snapshot = engine.state_handle();
                std::thread::Builder::new()
                    .name("hm-autosave".into())
                    .spawn(move || {
                        let mut last: Option<EngineState> = None;
                        loop {
                            std::thread::sleep(Duration::from_secs(2));
                            let current = (*snapshot.load_full()).clone();
                            if last.as_ref() == Some(&current) {
                                continue;
                            }
                            if let Ok(json) = serde_json::to_string_pretty(&current) {
                                // Write-then-rename so a crash can't leave a
                                // half-written settings file.
                                let tmp = path.with_extension("json.tmp");
                                if std::fs::write(&tmp, &json).is_ok() {
                                    let _ = std::fs::rename(&tmp, &path);
                                }
                            }
                            last = Some(current);
                        }
                    })
                    .ok();
            }

            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-frame-forwarder".into())
                .spawn(move || forward_frames(handle, meters, spectrum, pos, playing, paused))
                .ok();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_list_output_devices,
            commands::audio::audio_list_input_devices,
            commands::engine::engine_get_state,
            commands::engine::engine_set_power,
            commands::engine::engine_set_master_volume,
            commands::engine::engine_set_eq,
            commands::engine::engine_set_bass,
            commands::engine::engine_set_spatializer,
            commands::engine::engine_set_surround3d,
            commands::engine::player_play_file,
            commands::engine::player_play_radio,
            commands::engine::player_play_capture,
            commands::engine::player_play_system_audio,
            commands::engine::capture_virtual_available,
            commands::engine::system_audio_available,
            commands::engine::player_stop,
            commands::engine::player_pause,
            commands::engine::player_resume,
            commands::engine::player_seek,
            commands::engine::player_is_playing,
            commands::presets::eq_list_presets,
            commands::presets::eq_apply_preset,
            commands::presets::eq_save_custom,
            commands::presets::eq_update,
            commands::presets::eq_delete,
            commands::profiles::profile_list,
            commands::profiles::profile_set_active,
            commands::profiles::profile_clear,
            commands::library::library_scan,
            commands::library::library_list,
            commands::library::library_remove,
            commands::library::playlist_list,
            commands::library::playlist_create,
            commands::library::playlist_rename,
            commands::library::playlist_delete,
            commands::library::playlist_tracks,
            commands::library::playlist_add,
            commands::library::playlist_remove,
            commands::library::playlist_reorder,
            commands::radio::radio_search,
            commands::radio::radio_favorites_list,
            commands::radio::radio_favorite_add,
            commands::radio::radio_favorite_remove,
            commands::mixer::mixer_list_sessions,
            commands::mixer::mixer_set_volume,
            commands::mixer::mixer_set_muted,
            commands::license::license_status,
            commands::license::license_activate,
            commands::license::license_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the HypeMuzik application");
}
