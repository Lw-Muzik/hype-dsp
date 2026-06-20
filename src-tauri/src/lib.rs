//! HypeMuzik desktop — Tauri application entry point.
//!
//! Wires the internal crates (`hm-*`) to the webview UI: creates the audio
//! engine, registers plugins and the typed command handlers, spawns the
//! meter-frame forwarder, then runs the event loop. The heavy lifting (DSP,
//! audio engine, media, persistence) lives in the workspace crates; this layer
//! is the thin, well-documented bridge between Rust and React.

mod cloud;
mod cloud_meta;
mod commands;
mod control;
mod media;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use std::sync::Mutex;

use arc_swap::ArcSwap;
use hm_audio::{AudioEngine, EngineMeters, PlaybackPos, SpectrumTap, SPECTRUM_BANDS};
use hm_core::{EngineFrame, EngineState, LicenseMock, MediaStore, MeterFrame, PresetStore, TrackMeta};
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
    /// Whether the active source can be scrubbed (false for live radio).
    seekable: bool,
}

/// Emits real-time meter + spectrum frames to the UI at ~60 fps over the
/// `engine:frame` event, and play/stop transitions over `engine:transport`.
/// Runs for the app's lifetime on its own thread; it only reads lock-free
/// telemetry.
#[allow(clippy::too_many_arguments)]
fn forward_frames(
    app: tauri::AppHandle,
    meters: Arc<EngineMeters>,
    spectrum: Arc<SpectrumTap>,
    pos: Arc<PlaybackPos>,
    playing: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    track_meta: Arc<ArcSwap<TrackMeta>>,
    meta_version: Arc<AtomicU64>,
    queue_index: Arc<AtomicUsize>,
    media: media::MediaSession,
) {
    let mut last_playing = false;
    let mut last_paused = false;
    let mut last_meta_version = 0u64;
    let mut last_queue_index = usize::MAX;
    // Rounded duration last pushed to the OS, so we can re-publish metadata once
    // a stream's length becomes known (it's unknown at the first meta event).
    let mut last_media_dur: Option<u64> = None;
    let mut tick: u32 = 0;
    loop {
        std::thread::sleep(Duration::from_millis(16));
        tick = tick.wrapping_add(1);
        let now_playing = playing.load(Ordering::Relaxed);

        // Follow the gapless queue's current track index FIRST (it resets the
        // now-playing card for the new track)...
        let qi = queue_index.load(Ordering::Acquire);
        if qi != last_queue_index {
            last_queue_index = qi;
            let _ = app.emit("engine:queue_index", qi as u32);
        }

        // ...then the decoded track's tags + cover art refine it.
        let version = meta_version.load(Ordering::Acquire);
        let dur = pos.duration_secs();
        let dur_key = dur.map(|d| d.round() as u64);
        if version != last_meta_version {
            last_meta_version = version;
            let meta = (*track_meta.load_full()).clone();
            // Mirror the now-playing card to the OS media controls.
            media.set_metadata(
                meta.title.clone(),
                meta.artist.clone(),
                meta.album.clone(),
                meta.cover.clone(),
                dur,
            );
            last_media_dur = dur_key;
            let _ = app.emit("engine:now_playing", meta);
        } else if dur_key != last_media_dur && now_playing {
            // A stream just learned its length: re-publish so the OS scrubber
            // shows the right duration.
            last_media_dur = dur_key;
            let meta = (*track_meta.load_full()).clone();
            media.set_metadata(meta.title, meta.artist, meta.album, meta.cover, dur);
        }

        let now_paused = paused.load(Ordering::Relaxed);
        if now_playing != last_playing || now_paused != last_paused {
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
            }
            // Keep the OS play/pause indicator in sync with the engine.
            media.set_playback(now_playing, now_paused, pos.position_secs());
            last_playing = now_playing;
            last_paused = now_paused;
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
                        duration_secs: dur,
                        paused: now_paused,
                        seekable: pos.is_seekable(),
                    },
                );
            }
            // Re-sync the OS scrubber's elapsed position about once a second
            // (the system interpolates between updates from the playback rate).
            if tick % 64 == 0 {
                media.set_playback(true, now_paused, pos.position_secs());
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
    let track_meta = engine.track_meta_handle();
    let meta_version = engine.meta_version_handle();
    let queue_index = engine.queue_index_handle();

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

            // Account + real licensing against the Management API (the gate the
            // app actually enforces — replaces the local mock for access).
            let account_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("account.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_account.json"));
            app.manage(commands::account::AccountState::open(account_path));

            // Per-app mixer controller (real on Windows; unsupported stub on macOS).
            app.manage::<commands::mixer::Mixer>(Mutex::new(hm_platform::default_controller()));

            // Cloud music (Google Drive / Dropbox) token store.
            let cloud_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("cloud-tokens.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_cloud.json"));
            app.manage(cloud::CloudState::load(cloud_path));

            // Cloud track metadata (tags + cover) cache, so each cloud file's
            // ID3 is only downloaded once.
            let cloud_meta_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("cloud-meta.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_cloud_meta.json"));
            app.manage(cloud_meta::CloudMetaCache::load(cloud_meta_path));

            // MilkDrop visualizer sidecar process handle.
            app.manage(commands::visualizer::VisualizerState::default());

            // In-app (Canvas/WebGL) visualizer scene selection (persisted).
            let scenes_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("scenes.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_scenes.json"));
            app.manage(commands::scenes::SceneState::load(scenes_path));

            // Phone Link (stream the phone's library over the LAN) pairing store.
            let link_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("paired-devices.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_paired_devices.json"));
            app.manage(hm_link::LinkState::load(link_path));
            // Continuous phone discovery (streams `link:phone_found` events).
            app.manage(commands::link::DiscoveryState::default());

            // Remote (cross-network) phone link over iroh. When a phone pairs,
            // `on_paired` registers it into LinkState as a loopback proxy so its
            // library loads through the same path as a LAN phone, then notifies
            // the UI to refresh.
            let remote_dir = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d
                })
                .unwrap_or_else(|_| std::env::temp_dir());
            let pair_handle = app.handle().clone();
            match hm_remote::RemoteManager::new(
                remote_dir.join("remote-secret.bin"),
                remote_dir.join("remote-peers.json"),
                hm_link::device_name(),
                true,
                move |phone| {
                    pair_handle.state::<hm_link::LinkState>().register_remote(
                        phone.id.clone(),
                        phone.name.clone(),
                        phone.port,
                        phone.token.clone(),
                    );
                    let _ = pair_handle.emit("link:remote_connected", &phone.id);
                },
            ) {
                Ok(remote) => {
                    app.manage(remote);
                    // Silently redial known remote phones in the background so
                    // their libraries come back without blocking startup.
                    let bg = app.handle().clone();
                    std::thread::spawn(move || {
                        let remote = bg.state::<hm_remote::RemoteManager>();
                        let link = bg.state::<hm_link::LinkState>();
                        for phone in remote.connect_known() {
                            link.register_remote(
                                phone.id.clone(),
                                phone.name.clone(),
                                phone.port,
                                phone.token.clone(),
                            );
                            let _ = bg.emit("link:remote_connected", &phone.id);
                        }
                    });
                }
                Err(e) => eprintln!("remote phone link unavailable: {e}"),
            }

            // Phone Link cast: a control server phones can push tracks to, plus
            // an mDNS advertisement so they can find this desktop.
            control::start(app.handle().clone());

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

            // OS media controls (Control Center / SMTC / MPRIS). Forwards the
            // engine's transport to the OS and the OS's transport actions back
            // to the UI over the `media:command` event.
            let media_session = media::start(app.handle().clone());

            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-frame-forwarder".into())
                .spawn(move || {
                    forward_frames(
                        handle,
                        meters,
                        spectrum,
                        pos,
                        playing,
                        paused,
                        track_meta,
                        meta_version,
                        queue_index,
                        media_session,
                    )
                })
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
            commands::engine::engine_set_room,
            commands::cloud::cloud_status,
            commands::cloud::cloud_connect,
            commands::cloud::cloud_disconnect,
            commands::cloud::cloud_list,
            commands::cloud::cloud_all_audio,
            commands::cloud::cloud_track_metadata,
            commands::cloud::cloud_play,
            commands::link::link_discover,
            commands::link::link_paired,
            commands::link::link_pair,
            commands::link::link_pair_address,
            commands::link::link_discover_start,
            commands::link::link_discover_stop,
            commands::link::link_unpair,
            commands::link::link_remote_qr,
            commands::link::link_remote_cancel,
            commands::link::link_remote_status,
            commands::link::link_remote_connect,
            commands::link::link_remote_forget,
            commands::link::link_library,
            commands::link::link_artwork,
            commands::link::link_lyrics,
            commands::link::link_play,
            commands::engine::player_play_file,
            commands::engine::player_play_radio,
            commands::engine::player_play_queue,
            commands::engine::engine_set_playback,
            commands::engine::player_play_capture,
            commands::engine::player_play_system_audio,
            commands::engine::stop_system_audio,
            commands::visualizer::visualizer_available,
            commands::visualizer::visualizer_preset_names,
            commands::visualizer::visualizer_start,
            commands::visualizer::visualizer_set_preset,
            commands::visualizer::visualizer_stop,
            commands::visualizer::visualizer_is_open,
            commands::scenes::scene_list,
            commands::scenes::scene_selected,
            commands::scenes::scene_select,
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
            commands::library::library_refresh_tags,
            commands::library::library_list,
            commands::library::library_remove,
            commands::library::library_artwork,
            commands::identify::identify_track,
            commands::identify::library_identify_missing,
            commands::lyrics::lyrics_fetch,
            commands::library::playlist_list,
            commands::library::playlist_create,
            commands::library::playlist_rename,
            commands::library::playlist_delete,
            commands::library::playlist_tracks,
            commands::library::playlist_add,
            commands::library::playlist_remove,
            commands::library::playlist_reorder,
            commands::radio::radio_search,
            commands::radio::radio_african_countries,
            commands::radio::radio_by_country,
            commands::radio::radio_favorites_list,
            commands::radio::radio_favorite_add,
            commands::radio::radio_favorite_remove,
            commands::mixer::mixer_list_sessions,
            commands::mixer::mixer_set_volume,
            commands::mixer::mixer_set_muted,
            commands::license::license_status,
            commands::license::license_activate,
            commands::license::license_deactivate,
            commands::account::account_status,
            commands::account::account_login,
            commands::account::account_signup,
            commands::account::account_logout,
            commands::account::account_heartbeat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the HypeMuzik application");
}
