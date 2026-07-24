//! HypeMuzik desktop — Tauri application entry point.
//!
//! Wires the internal crates (`hm-*`) to the webview UI: creates the audio
//! engine, registers plugins and the typed command handlers, spawns the
//! meter-frame forwarder, then runs the event loop. The heavy lifting (DSP,
//! audio engine, media, persistence) lives in the workspace crates; this layer
//! is the thin, well-documented bridge between Rust and React.

mod cloud;
mod cloud_list;
mod cloud_meta;
mod commands;
mod control;
mod media;
mod tv_proxy;
mod updater;
mod ytmusic;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use std::sync::Mutex;

use arc_swap::ArcSwap;
use hm_audio::{AudioEngine, CompanderMeter, EngineMeters, PlaybackPos, SpectrumTap, SPECTRUM_BANDS};
use hm_core::{ChainPresetStore, EngineFrame, EngineState, LicenseMock, MediaStore, MeterFrame, PresetStore, TrackMeta};
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
    /// Whether the active source is currently buffering (waiting for network).
    buffering: bool,
    /// Latest download throughput estimate from the active source, bytes/sec.
    download_bps: u64,
    /// Mid-track rebuffer event count from the active source.
    rebuffer_count: u32,
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
    compander_gr: Arc<CompanderMeter>,
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
        // Idle backoff: when the transport is inactive nothing below emits (the
        // settle frame fires on the stop transition itself, which is detected at
        // the fast cadence because `last_playing` is still true on that tick),
        // so a 16 ms tick would only buy faster *start* detection. 150 ms keeps
        // that imperceptible while cutting the always-on wakeups ~10× — and
        // "idle in the background" is the app's common state, since closing the
        // window hides it rather than quitting.
        std::thread::sleep(if last_playing {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(150)
        });
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
                            compander_gr: None,
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
            // Meters/spectrum/compander telemetry at ~30 fps (every other 16 ms
            // tick), not ~60 — plenty smooth for the visualizers and it halves
            // both the IPC traffic and the React re-renders it drives. Transport
            // transitions above are still detected every tick, so latency is
            // unchanged; only the steady visual-telemetry rate drops.
            if tick % 2 == 0 {
                let _ = app.emit(
                    "engine:frame",
                    EngineFrame {
                        meters: meters.load(),
                        spectrum: Some(spectrum.load()),
                        compander_gr: Some(compander_gr.load().to_vec()),
                    },
                );
            }
            // Transport progress at ~10 fps (every ~6 ticks).
            if tick % 6 == 0 {
                let _ = app.emit(
                    "engine:progress",
                    Progress {
                        position_secs: pos.position_secs(),
                        duration_secs: dur,
                        paused: now_paused,
                        seekable: pos.is_seekable(),
                        buffering: pos.is_buffering(),
                        download_bps: pos.download_bps(),
                        rebuffer_count: pos.rebuffer_count(),
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
    let compander_gr = engine.compander_gr();
    let pos = engine.pos();
    let playing = engine.playing_flag();
    let paused = engine.paused_flag();
    let track_meta = engine.track_meta_handle();
    let meta_version = engine.meta_version_handle();
    let queue_index = engine.queue_index_handle();

    tauri::Builder::default()
        // MUST be the first plugin: a second launch (e.g. opening an audio file
        // from the file manager while the app runs) forwards its argv here and
        // exits, instead of spawning a duplicate. We pull the audio paths out,
        // hand them to the running UI, and focus the window. (macOS routes file
        // opens through `RunEvent::Opened` instead — see the run loop below.)
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths = commands::open_with::audio_paths(argv.into_iter().skip(1));
            if !paths.is_empty() {
                app.state::<commands::open_with::PendingOpen>().push(paths.clone());
                let _ = app.emit("app:open_files", paths);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        // Auto-update. The plugin only supplies the machinery; *when* anything
        // happens is decided in `updater.rs` — checked on a cadence, downloaded
        // in the background, and installed at quit, which is the one moment the
        // audio tap is already torn down.
        //
        // Note the plugin wires its own `on_before_exit` to Tauri's
        // `cleanup_before_exit`; that hook lives on `UpdaterBuilder`, not here.
        // Our teardown runs explicitly before `install`, so it does not depend
        // on that.
        .plugin(tauri_plugin_updater::Builder::new().build())
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
        // Closing the window (the red traffic-light button / window "X") hides
        // the app instead of quitting, so playback and the audio engine keep
        // running in the background. It comes back via the dock icon (macOS,
        // see `RunEvent::Reopen` below), relaunching the app, or "Open With".
        // ⌘Q (the Quit menu item) still exits normally.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(engine)
        .manage(updater::UpdaterState::default())
        .manage(commands::open_with::PendingOpen::default())
        .setup(move |app| {
            // Audio files passed on the command line (Windows/Linux cold launch,
            // or "Open With" before the window mounts) — buffer them for the UI
            // to drain on init. macOS delivers these via `RunEvent::Opened`.
            app.state::<commands::open_with::PendingOpen>()
                .push(std::env::args().skip(1));

            // In-app TV: a local HLS proxy so channels play in an embedded
            // `<video>` (hls.js) — it launders CSP/CORS/custom-header/mixed-content
            // so no native window and no external player are needed.
            if let Some(proxy) = tv_proxy::start() {
                app.manage(proxy);
            }
            app.manage(hm_media::tv::TvHealthCache::default());

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

            // Whole-chain DSP preset store (JSON file).  Unlike the EQ-only
            // PresetStore this holds complete EngineState snapshots. Always
            // succeeds: ChainPresetStore::open never does I/O itself.
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let chain_store = ChainPresetStore::open(&dir.join("chain-presets.json"));
                app.manage(Mutex::new(chain_store));
            } else {
                // Fallback to a temp-dir-backed store so the app still runs.
                let chain_store =
                    ChainPresetStore::open(&std::env::temp_dir().join("hm_chain_presets.json"));
                app.manage(Mutex::new(chain_store));
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

            // Stem separation (htdemucs ONNX, CoreML-accelerated). The model
            // (htdemucs.onnx + sidecar .json) and the ONNX Runtime dylib are
            // placed under <app data>/stems by scripts/get_stems_model.sh;
            // separated stems cache per track.
            let stem_root = app
                .path()
                .app_data_dir()
                .map(|d| d.join("stems"))
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_stems"));
            let _ = std::fs::create_dir_all(stem_root.join("cache"));
            // Point `ort` (load-dynamic) at a libonnxruntime placed next to the
            // model; otherwise it searches the system library path.
            let dylib_name = if cfg!(windows) {
                "onnxruntime.dll"
            } else if cfg!(target_os = "macos") {
                "libonnxruntime.dylib"
            } else {
                "libonnxruntime.so"
            };
            let dylib = stem_root.join(dylib_name);
            if dylib.is_file() {
                // Set once during single-threaded setup, before any separator
                // session is created.
                std::env::set_var("ORT_DYLIB_PATH", &dylib);
            }
            app.manage(commands::stems::StemState {
                separator: hm_stems::Separator::new(
                    stem_root.join("model"),
                    stem_root.join("cache"),
                ),
            });

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

            // Cloud account listing cache, so reopening the app serves the
            // library instantly instead of re-walking the account over the wire.
            // Constructed without touching the disk (the file can be MBs) and
            // warmed on a background thread; commands that beat the warmer just
            // trigger the same one-time load themselves.
            let cloud_list_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("cloud-list.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_cloud_list.json"));
            app.manage(cloud_list::CloudListCache::new(cloud_list_path));
            let cloud_list_warm = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-cloudlist-warm".into())
                .spawn(move || {
                    cloud_list_warm.state::<cloud_list::CloudListCache>().warm();
                })
                .ok();

            // YouTube Music. The session (Google cookies — full account
            // credentials) lives in the OS keychain, not next to these JSON
            // stores; `load` restores it, or falls back to signed-out if the
            // keychain is locked or unavailable.
            //
            // Register the app-managed tools dir *first*: it's where the
            // one-click setup installs yt-dlp/ffmpeg, and detection checks it
            // ahead of PATH. Then let the background updater keep that copy
            // current (no-op unless the managed copy is the active one).
            if let Ok(dir) = app.path().app_local_data_dir() {
                hm_ytmusic::ytdlp::set_managed_bin_dir(dir.join("bin"));
            }
            commands::ytmusic_setup::spawn_auto_update();
            app.manage(hm_ytmusic::YtMusicState::load());

            // Yesterday's stream urls are good for ~6 hours; restoring them
            // makes relaunch-and-play cost one ~300ms probe instead of a ~5s
            // yt-dlp resolve. Quarantined until probed — see hm-ytmusic.
            if let Some(path) = ytmusic::url_cache_path(app.handle()) {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    app.state::<hm_ytmusic::YtMusicState>().restore_url_cache(&json);
                }
                // Save on a slow heartbeat, only when something changed. The
                // entries are worth at most ~6h, so losing the tail on a crash
                // costs one resolve — no need for write-on-every-change. Plain
                // OS thread + blocking sleep, matching the other periodic
                // background tasks in this file (media polling, EQ autosave)
                // rather than an async task.
                let handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("hm-ytcache-saver".into())
                    .spawn(move || {
                        let mut last_saved: u64 = 0;
                        loop {
                            std::thread::sleep(Duration::from_secs(60));
                            let state = handle.state::<hm_ytmusic::YtMusicState>();
                            let generation = state.url_cache_generation();
                            if generation == last_saved {
                                continue;
                            }
                            if let Some((g, json)) = state.url_cache_snapshot() {
                                ytmusic::save_url_cache(&path, &json);
                                last_saved = g;
                            }
                        }
                    })
                    .ok();
            }

            let yt_settings_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("ytmusic-settings.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_ytmusic_settings.json"));
            // Downloads default under the OS music dir; home, then temp, are
            // last-resort fallbacks so a download always has somewhere to land.
            let music_dir = app
                .path()
                .audio_dir()
                .or_else(|_| app.path().home_dir())
                .unwrap_or_else(|_| std::env::temp_dir());
            app.manage(ytmusic::YtSettings::load(yt_settings_path, music_dir));

            // Cached playlist listing, so relaunching shows the library instantly
            // instead of re-walking every playlist. Same lazy-load + background
            // warm as the cloud listing, for the same reason.
            let yt_lib_path = app
                .path()
                .app_data_dir()
                .map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("ytmusic-library.json")
                })
                .unwrap_or_else(|_| std::env::temp_dir().join("hm_ytmusic_library.json"));
            app.manage(ytmusic::YtLibraryCache::new(yt_lib_path));
            let yt_warm = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-ytlib-warm".into())
                .spawn(move || {
                    yt_warm.state::<ytmusic::YtLibraryCache>().warm();
                })
                .ok();

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
            // `on_paired` (wired inside `RemoteState::manager`) registers it
            // into LinkState as a loopback proxy so its library loads through
            // the same path as a LAN phone, then notifies the UI to refresh.
            //
            // The manager itself (a dedicated tokio runtime + a relay-connected
            // iroh endpoint) is NOT built here: remote commands build it on
            // first use, and the background thread below builds it at startup
            // only when phones were previously paired — so they silently
            // reconnect exactly as before, without ever blocking setup.
            let remote_dir = app
                .path()
                .app_data_dir()
                .inspect(|d| {
                    let _ = std::fs::create_dir_all(d);
                })
                .unwrap_or_else(|_| std::env::temp_dir());
            app.manage(commands::link::RemoteState::new(
                app.handle().clone(),
                remote_dir.join("remote-secret.bin"),
                remote_dir.join("remote-peers.json"),
            ));
            let bg = app.handle().clone();
            std::thread::Builder::new()
                .name("hm-remote-reconnect".into())
                .spawn(move || {
                    let state = bg.state::<commands::link::RemoteState>();
                    if !state.has_known_peers() {
                        return;
                    }
                    // Known phones exist: build the manager now (off the main
                    // thread) and silently redial them so their libraries come
                    // back without user action.
                    let Ok(remote) = state.manager() else { return };
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
                })
                .ok();

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
// Background update checks. Deliberately after the state restore so
                // the first check can never race the settings the user is about
                // to see, and delayed further inside `spawn_cadence`.
                updater::spawn_cadence(app.handle().clone());

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
                        compander_gr,
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

            // Linux: if system-wide EQ is unavailable only because a package is
            // missing (e.g. pulseaudio-utils on a PipeWire-less box), auto-install
            // it behind a single polkit prompt. No-op when nothing is needed
            // (the PipeWire majority) or the user already declined.
            #[cfg(target_os = "linux")]
            commands::linux_audio_setup::auto_setup_on_launch(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_output_devices,
            commands::audio::audio_set_default_output,
            commands::audio::audio_list_input_devices,
            commands::engine::engine_get_state,
            commands::engine::engine_set_power,
            commands::engine::engine_set_master_volume,
            commands::engine::engine_set_eq,
            commands::engine::engine_eq_import_graphic,
            commands::engine::engine_eq_import_vdc,
            commands::engine::ddc_list,
            commands::engine::engine_eq_apply_ddc,
            commands::autoeq::autoeq_search,
            commands::autoeq::autoeq_fetch_apply,
            commands::engine::engine_set_bass,
            commands::engine::engine_set_spatializer,
            commands::engine::engine_set_surround3d,
            commands::engine::engine_set_room,
            commands::engine::engine_set_convolver,
            commands::engine::engine_set_compander,
            commands::engine::engine_set_saturation,
            updater::updater_status,
            updater::updater_check_now,
            updater::updater_restart_now,
            commands::engine::engine_script_compile,
            commands::engine::engine_set_script,
            commands::engine::engine_set_output,
            commands::engine::engine_convolver_load_ir,
            commands::cloud::cloud_status,
            commands::cloud::cloud_connect,
            commands::cloud::cloud_disconnect,
            commands::cloud::cloud_list,
            commands::cloud::cloud_all_audio,
            commands::cloud::cloud_cached_tags,
            commands::cloud::cloud_track_metadata,
            commands::cloud::cloud_track_tags,
            commands::cloud::cloud_track_cover,
            commands::cloud::cloud_play,
            commands::cloud::player_play_cloud_queue,
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
            commands::link::link_play_queue,
            commands::ytmusic::ytmusic_status,
            commands::ytmusic_setup::ytmusic_setup,
            commands::ytmusic::ytmusic_sign_in,
            commands::ytmusic::ytmusic_sign_out,
            commands::ytmusic::ytmusic_all_tracks,
            commands::ytmusic::ytmusic_explore_categories,
            commands::ytmusic::ytmusic_explore_page,
            commands::ytmusic::ytmusic_explore_tracks,
            commands::ytmusic::ytmusic_search,
            commands::ytmusic::ytmusic_search_suggestions,
            commands::ytmusic::ytmusic_artist_page,
            commands::ytmusic::ytmusic_radio,
            commands::ytmusic::ytmusic_radio_continue,
            commands::ytmusic::ytmusic_video_url,
            commands::ytmusic::ytmusic_prefetch,
            commands::ytmusic::ytmusic_prefetch_batch,
            commands::ytmusic::ytmusic_video_prefetch,
            commands::ytmusic::ytmusic_play,
            commands::ytmusic::player_play_ytmusic_queue,
            commands::ytmusic::ytmusic_download,
            commands::ytmusic::ytmusic_download_to_phone,
            commands::ytmusic::ytmusic_download_dir,
            commands::ytmusic::ytmusic_set_download_dir,
            commands::link::link_upload,
            commands::engine::player_play_file,
            commands::engine::player_play_radio,
            commands::engine::player_play_queue,
            commands::engine::engine_set_playback,
            commands::engine::engine_set_data_saver,
            commands::engine::engine_set_autoplay,
            commands::engine::player_play_capture,
            commands::engine::player_play_system_audio,
            commands::engine::stop_system_audio,
            commands::engine::system_eq_set_scope,
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
            commands::engine::system_audio_status,
            commands::engine::system_eq_status,
            commands::engine::system_audio_install_driver,
            commands::cable::system_audio_setup_routing,
            commands::linux_audio_setup::linux_system_audio_setup,
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
            commands::chain_presets::chain_preset_list,
            commands::chain_presets::chain_preset_save,
            commands::chain_presets::chain_preset_apply,
            commands::chain_presets::chain_preset_delete,
            commands::chain_presets::chain_preset_export,
            commands::chain_presets::chain_preset_import,
            commands::profiles::profile_list,
            commands::profiles::profile_set_active,
            commands::profiles::profile_clear,
            commands::library::library_scan,
            commands::library::library_refresh_tags,
            commands::library::library_list,
            commands::library::library_count,
            commands::library::library_available_count,
            commands::library::library_list_page,
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
            commands::tv::tv_search,
            commands::tv::tv_check_alive,
            commands::tv::tv_by_country,
            commands::tv::tv_by_category,
            commands::tv::tv_categories,
            commands::tv::tv_countries,
            commands::tv::tv_favorites_list,
            commands::tv::tv_favorite_add,
            commands::tv::tv_favorite_remove,
            commands::tv::tv_stream_url,
            commands::mixer::mixer_list_sessions,
            commands::mixer::mixer_set_volume,
            commands::mixer::mixer_set_muted,
            commands::license::license_status,
            commands::license::license_activate,
            commands::license::license_deactivate,
            commands::account::account_status,
            commands::account::account_signup,
            commands::account::account_request_otp,
            commands::account::account_verify,
            commands::account::account_logout,
            commands::account::account_heartbeat,
            commands::stems::stems_status,
            commands::stems::stems_arm,
            commands::stems::stems_set_gain,
            commands::stems::stems_reset,
            commands::stems::stems_gains,
            commands::open_with::open_files,
            commands::open_with::take_pending_open,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the HypeMuzik application")
        .run(|_app, _event| {
            // Install a staged update on the way out. This is the ⌘Q / quit
            // path — window close only hides this app, so it is genuinely the
            // moment nothing is playing and the tap is already going away.
            if let tauri::RunEvent::ExitRequested { .. } = &_event {
                updater::install_on_exit(_app);

                // Flush the stream-url cache: the 60s heartbeat may owe a write.
                if let Some(path) = ytmusic::url_cache_path(_app) {
                    if let Some((_, json)) =
                        _app.state::<hm_ytmusic::YtMusicState>().url_cache_snapshot()
                    {
                        ytmusic::save_url_cache(&path, &json);
                    }
                }
            }

            // macOS delivers file-manager opens (and "Open With") as an Apple
            // "open documents" event, both at cold launch and while running —
            // never as argv. Buffer the audio paths for the UI to drain on init
            // and emit a warm event so an already-open app plays them at once.
            // (`RunEvent::Opened` only exists on macOS/iOS, hence the cfg gate;
            // Windows/Linux take the argv + single-instance paths above.)
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = &_event {
                let paths = commands::open_with::audio_paths(
                    urls.iter()
                        .filter_map(|u| u.to_file_path().ok())
                        .map(|p| p.to_string_lossy().into_owned()),
                );
                if !paths.is_empty() {
                    _app.state::<commands::open_with::PendingOpen>().push(paths.clone());
                    let _ = _app.emit("app:open_files", paths);
                    if let Some(window) = _app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }

            // Clicking the dock icon after the close button hid the window
            // re-shows it — the standard macOS "close hides, ⌘Q quits" pattern.
            // `RunEvent::Reopen` fires on dock activation and only exists on macOS.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &_event {
                if let Some(window) = _app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });

    // The app's event loop has ended. ONNX Runtime (the stem separator) aborts
    // ("mutex lock failed") if its global environment is torn down by C++ static
    // destructors during normal process exit. We're done — skip those
    // destructors entirely with `_exit` (state is already autosaved). `exit`
    // would still run them, so this must be `_exit`.
    unsafe { libc::_exit(0) };
}
