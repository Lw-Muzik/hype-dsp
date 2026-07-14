//! Native TV playback commands — drive the mpv window via [`hm_video`].
//!
//! A TV channel is video, so it can't run through the [`AudioEngine`]. Starting
//! one pauses the audio engine (so radio/local audio doesn't play underneath)
//! and hands the stream to a native mpv window with its built-in controls. Only
//! one channel plays at a time.

use std::path::PathBuf;

use hm_audio::AudioEngine;
use hm_core::{IpcError, TvChannel};
use hm_video::{PlayRequest, VideoPlayer};
use tauri::{AppHandle, Manager, State};

/// Locate the mpv binary: the bundled resource in a packaged app, else `mpv`
/// on `PATH` for development.
pub fn resolve_mpv(app: &AppHandle) -> PathBuf {
    if let Ok(res) = app.path().resource_dir() {
        // Bundled by `scripts/get_mpv.sh` into `resources/mpv/` (see the
        // `bundle.resources` "mpv/" mapping in tauri.conf.json).
        for candidate in ["mpv/mpv", "mpv/mpv.exe", "mpv", "mpv.exe"] {
            let path = res.join(candidate);
            if path.exists() {
                return path;
            }
        }
    }
    PathBuf::from("mpv")
}

/// Play (or switch to) a TV channel in the native window.
#[tauri::command(async)]
pub fn tv_play(
    engine: State<'_, AudioEngine>,
    player: State<'_, VideoPlayer>,
    channel: TvChannel,
) -> Result<(), IpcError> {
    // Don't stack TV video audio on top of anything the engine is playing.
    engine.pause();
    player
        .play(PlayRequest {
            url: channel.url,
            title: channel.name,
            user_agent: channel.user_agent,
            referrer: channel.referrer,
        })
        .map_err(|e| IpcError::new("tv_play_failed", e.to_string()))
}

/// Stop TV playback and close the native window.
#[tauri::command(async)]
pub fn tv_stop(player: State<'_, VideoPlayer>) {
    player.stop();
}

/// Whether a TV channel is currently playing (false once the user closes the
/// mpv window) — the UI polls this to clear its "now watching" indicator.
#[tauri::command(async)]
pub fn tv_player_status(player: State<'_, VideoPlayer>) -> bool {
    player.is_running()
}
