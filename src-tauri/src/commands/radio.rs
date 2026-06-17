//! Internet radio directory + favorites commands.

use hm_core::{IpcError, MediaStore, RadioStation};
use hm_media::radio;
use tauri::State;

/// Search the radio directory (falls back to the bundled seed when offline).
#[tauri::command]
pub fn radio_search(query: String) -> Vec<RadioStation> {
    radio::search(&query)
}

/// Favorited stations (persisted).
#[tauri::command]
pub fn radio_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<RadioStation>, IpcError> {
    store.list_favorites().map_err(Into::into)
}

#[tauri::command]
pub fn radio_favorite_add(
    store: State<'_, MediaStore>,
    station: RadioStation,
) -> Result<(), IpcError> {
    store.add_favorite(&station).map_err(Into::into)
}

#[tauri::command]
pub fn radio_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError> {
    store.remove_favorite(&id).map_err(Into::into)
}
