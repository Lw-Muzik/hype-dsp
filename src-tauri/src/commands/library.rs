//! Local library scan + playlist commands (SQLite-backed).

use std::path::Path;

use hm_audio::probe_duration;
use hm_core::{IpcError, LibraryTrack, MediaStore, Playlist};
use tauri::State;

/// Supported audio file extensions for library scanning.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "oga", "m4a", "aac", "mp4", "opus",
];
const MAX_DEPTH: usize = 8;

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn scan_dir(dir: &Path, store: &MediaStore, count: &mut usize, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, store, count, depth + 1);
        } else if is_audio(&path) {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            let track = LibraryTrack {
                path: path.to_string_lossy().into_owned(),
                title,
                artist: None,
                album: None,
                duration_secs: probe_duration(&path),
            };
            if store.upsert_track(&track).is_ok() {
                *count += 1;
            }
        }
    }
}

/// Recursively scan a folder for audio files and add them to the library.
/// Returns the number of tracks scanned.
#[tauri::command]
pub fn library_scan(store: State<'_, MediaStore>, dir: String) -> Result<usize, IpcError> {
    let path = Path::new(&dir);
    if !path.is_dir() {
        return Err(IpcError::new("invalid", "not a directory"));
    }
    let mut count = 0;
    scan_dir(path, &store, &mut count, 0);
    Ok(count)
}

#[tauri::command]
pub fn library_list(store: State<'_, MediaStore>) -> Result<Vec<LibraryTrack>, IpcError> {
    store.list_tracks().map_err(Into::into)
}

#[tauri::command]
pub fn library_remove(store: State<'_, MediaStore>, path: String) -> Result<(), IpcError> {
    store.remove_track(&path).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_list(store: State<'_, MediaStore>) -> Result<Vec<Playlist>, IpcError> {
    store.list_playlists().map_err(Into::into)
}

#[tauri::command]
pub fn playlist_create(store: State<'_, MediaStore>, name: String) -> Result<Playlist, IpcError> {
    store.create_playlist(&name).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_rename(
    store: State<'_, MediaStore>,
    id: String,
    name: String,
) -> Result<(), IpcError> {
    store.rename_playlist(&id, &name).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_delete(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError> {
    store.delete_playlist(&id).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_tracks(
    store: State<'_, MediaStore>,
    id: String,
) -> Result<Vec<LibraryTrack>, IpcError> {
    store.playlist_tracks(&id).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_add(
    store: State<'_, MediaStore>,
    id: String,
    path: String,
) -> Result<(), IpcError> {
    store.add_to_playlist(&id, &path).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_remove(
    store: State<'_, MediaStore>,
    id: String,
    path: String,
) -> Result<(), IpcError> {
    store.remove_from_playlist(&id, &path).map_err(Into::into)
}

#[tauri::command]
pub fn playlist_reorder(
    store: State<'_, MediaStore>,
    id: String,
    paths: Vec<String>,
) -> Result<(), IpcError> {
    store.reorder_playlist(&id, &paths).map_err(Into::into)
}
