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

use hm_audio::{AudioEngine, EngineMeters, SpectrumTap, SPECTRUM_BANDS};
use hm_core::{EngineFrame, MeterFrame, PresetStore};
use tauri::{Emitter, Manager};

/// Emits real-time meter + spectrum frames to the UI at ~60 fps over the
/// `engine:frame` event, and play/stop transitions over `engine:transport`.
/// Runs for the app's lifetime on its own thread; it only reads lock-free
/// telemetry.
fn forward_frames(
    app: tauri::AppHandle,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    playing: Arc<AtomicBool>,
) {
    let mut last_playing = false;
    loop {
        std::thread::sleep(Duration::from_millis(16));
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
        }
    }
}

/// Build and run the Tauri application.
pub fn run() {
    let engine = AudioEngine::new();
    let meters = engine.meters();
    let spectrum = engine.spectrum();
    let playing = engine.playing_flag();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
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

            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-frame-forwarder".into())
                .spawn(move || forward_frames(handle, meters, spectrum, playing))
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
            commands::engine::player_play_file,
            commands::engine::player_stop,
            commands::engine::player_is_playing,
            commands::presets::eq_list_presets,
            commands::presets::eq_apply_preset,
            commands::presets::eq_save_custom,
            commands::presets::eq_update,
            commands::presets::eq_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the HypeMuzik application");
}
