//! World TV directory + favorites commands (iptv-org).
//!
//! The TV counterpart to [`radio`](super::radio): browse channels by country or
//! category, search the global catalog, and persist favorites. Playback of a
//! chosen channel is handled separately by [`super::video`] (a native mpv
//! window), since TV is video and cannot go through the audio engine.

use std::path::PathBuf;

use hm_core::{IpcError, MediaStore, TvCategory, TvChannel, TvCountry};
use hm_media::tv;
use tauri::{AppHandle, Manager, State};

/// The directory playlists are cached under (the app data dir). `None` in the
/// unlikely event it can't be resolved — the directory falls back to
/// network-only, still functional.
fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

/// Search the global TV directory (falls back to the bundled seed offline).
#[tauri::command(async)]
pub fn tv_search(app: AppHandle, query: String) -> Vec<TvChannel> {
    tv::search(&query, cache_dir(&app).as_deref())
}

/// Every channel for a country (ISO 3166-1 alpha-2 code).
#[tauri::command(async)]
pub fn tv_by_country(app: AppHandle, code: String) -> Vec<TvChannel> {
    tv::by_country(&code, cache_dir(&app).as_deref())
}

/// Every channel for a category (iptv-org slug, e.g. "news").
#[tauri::command(async)]
pub fn tv_by_category(app: AppHandle, id: String) -> Vec<TvChannel> {
    tv::by_category(&id, cache_dir(&app).as_deref())
}

/// The browsable TV categories.
#[tauri::command]
pub fn tv_categories() -> Vec<TvCategory> {
    tv::categories()
}

/// The world country list for the browse grid.
#[tauri::command]
pub fn tv_countries() -> Vec<TvCountry> {
    tv::world_countries()
}

// Favorites take the shared `MediaStore` mutex (which a library scan can hold
// for whole write batches), so they run `(async)` off the webview main thread —
// same discipline as the radio favorites commands.
#[tauri::command(async)]
pub fn tv_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<TvChannel>, IpcError> {
    store.list_tv_favorites().map_err(Into::into)
}

#[tauri::command(async)]
pub fn tv_favorite_add(store: State<'_, MediaStore>, channel: TvChannel) -> Result<(), IpcError> {
    store.add_tv_favorite(&channel).map_err(Into::into)
}

#[tauri::command(async)]
pub fn tv_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError> {
    store.remove_tv_favorite(&id).map_err(Into::into)
}
