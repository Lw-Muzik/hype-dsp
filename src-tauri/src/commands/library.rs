//! Local library scan + playlist commands (SQLite-backed).

use std::path::{Path, PathBuf};

use hm_audio::{probe_artwork, probe_track};
use hm_core::{IpcError, LibraryTrack, MediaStore, Playlist};
use serde::Serialize;
use tauri::{Emitter, State};

/// Supported audio file extensions for library scanning.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "oga", "m4a", "aac", "mp4", "opus",
];
const MAX_DEPTH: usize = 8;
/// Tracks tagged + written per transaction. Big enough to amortize fsync,
/// small enough to keep progress smooth and memory bounded over huge libraries.
const SCAN_CHUNK: usize = 256;

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Scan progress, emitted on `library:scan_progress` so the UI can show it
/// instead of appearing frozen during a large import.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    done: usize,
    total: usize,
}

/// Collect audio file paths under `dir` (cheap directory walk, no decoding).
fn collect_audio_paths(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_audio_paths(&path, out, depth + 1);
        } else if is_audio(&path) {
            out.push(path);
        }
    }
}

/// Read tags + duration for `paths` and upsert them in batched transactions,
/// emitting `library:scan_progress` as it goes. Shared by the folder scan and
/// the tag refresh. Returns the number indexed.
fn index_paths(app: &tauri::AppHandle, store: &MediaStore, paths: &[PathBuf]) -> usize {
    let total = paths.len();
    let _ = app.emit("library:scan_progress", ScanProgress { done: 0, total });
    let mut done = 0;
    for chunk in paths.chunks(SCAN_CHUNK) {
        let batch: Vec<LibraryTrack> = chunk
            .iter()
            .map(|p| {
                // One file open for both tags and duration — the same extractor
                // the now-playing card uses, so the listing matches it.
                let (tags, duration) = probe_track(p);
                let filename = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                LibraryTrack {
                    path: p.to_string_lossy().into_owned(),
                    title: tags.title.filter(|t| !t.trim().is_empty()).unwrap_or(filename),
                    artist: tags.artist,
                    album: tags.album,
                    genre: tags.genre,
                    duration_secs: duration,
                }
            })
            .collect();
        if store.upsert_tracks(&batch).is_ok() {
            done += batch.len();
        }
        let _ = app.emit("library:scan_progress", ScanProgress { done, total });
    }
    done
}

/// Recursively scan a folder for audio files and add them to the library,
/// reading each file's tags. Designed to stay smooth over very large libraries:
/// it walks first, then tags + writes in batched transactions, emitting
/// `library:scan_progress` along the way. Runs off the UI thread (Tauri command
/// pool). Returns the number of tracks imported.
#[tauri::command]
pub fn library_scan(
    app: tauri::AppHandle,
    store: State<'_, MediaStore>,
    dir: String,
) -> Result<usize, IpcError> {
    let path = Path::new(&dir);
    if !path.is_dir() {
        return Err(IpcError::new("invalid", "not a directory"));
    }
    let mut paths = Vec::new();
    collect_audio_paths(path, &mut paths, 0);
    Ok(index_paths(&app, &store, &paths))
}

/// Re-read tags + duration for every track already in the library, updating
/// rows in place. Use this to backfill tags for a library that was scanned
/// before tag extraction existed (or after files were re-tagged) without
/// re-picking folders. Returns the number refreshed.
#[tauri::command]
pub fn library_refresh_tags(
    app: tauri::AppHandle,
    store: State<'_, MediaStore>,
) -> Result<usize, IpcError> {
    let paths: Vec<PathBuf> = store
        .list_tracks()
        .map_err(IpcError::from)?
        .into_iter()
        .map(|t| PathBuf::from(t.path))
        .collect();
    Ok(index_paths(&app, &store, &paths))
}

#[tauri::command]
pub fn library_list(store: State<'_, MediaStore>) -> Result<Vec<LibraryTrack>, IpcError> {
    store.list_tracks().map_err(Into::into)
}

#[tauri::command]
pub fn library_remove(store: State<'_, MediaStore>, path: String) -> Result<(), IpcError> {
    store.remove_track(&path).map_err(Into::into)
}

/// A track's embedded cover art as a `data:` URI, or `None` if it has none.
/// Read on demand (the scan skips artwork to stay fast), so the UI lazy-loads
/// covers only for the rows/cards it actually shows. Never errors.
#[tauri::command]
pub fn library_artwork(path: String) -> Option<String> {
    probe_artwork(Path::new(&path))
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
