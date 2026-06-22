//! Opening audio files from the OS file manager.
//!
//! Files reach the app three ways (see `lib.rs`): macOS delivers them as
//! `RunEvent::Opened`, Windows/Linux as launch argv, and a second launch while
//! running is forwarded by the single-instance plugin. All of them funnel paths
//! into [`PendingOpen`]; the front end drains the buffer on mount (covering the
//! cold-launch race where the webview isn't ready yet) and also listens for the
//! `app:open_files` event (warm opens). For each path the front end calls
//! [`open_files`], which imports the file into the library and hands back the
//! resolved track to play. Supported types come from `library::is_audio`.

use std::path::PathBuf;
use std::sync::Mutex;

use hm_core::{LibraryTrack, MediaStore};
use tauri::State;

use crate::commands::library::{is_audio, track_from_path};

/// Keep only the audio files from a set of raw OS arguments/urls (drops flags
/// and unrelated paths). Used for the warm-open event payload; [`PendingOpen`]
/// filters again on its own so a raw push is always safe.
pub fn audio_paths<I: IntoIterator<Item = String>>(paths: I) -> Vec<String> {
    paths
        .into_iter()
        .filter(|p| is_audio(&PathBuf::from(p)))
        .collect()
}

/// Audio paths handed to the app by the OS before the front end is ready to
/// receive them. Drained once via [`take_pending_open`] when the UI mounts.
#[derive(Default)]
pub struct PendingOpen(Mutex<Vec<String>>);

impl PendingOpen {
    /// Buffer audio paths for the front end to drain on mount. Non-audio and
    /// duplicate entries are dropped so a stray argument can't enqueue junk.
    pub fn push(&self, paths: impl IntoIterator<Item = String>) {
        let mut buf = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for p in paths {
            if is_audio(&PathBuf::from(&p)) && !buf.contains(&p) {
                buf.push(p);
            }
        }
    }

    fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Drain any audio paths the OS handed us before the UI was ready (cold launch
/// / "Open With" before the window mounted). Called once when the front end
/// initialises; returns `[]` when nothing is pending.
#[tauri::command]
pub fn take_pending_open(pending: State<'_, PendingOpen>) -> Vec<String> {
    pending.drain()
}

/// Import `paths` into the library (reading each file's tags) and return the
/// resolved tracks so the front end can enqueue and play them. Filters out
/// non-audio and unreadable paths; the returned order matches the input (minus
/// dropped entries) so the caller can play the first and queue the rest.
#[tauri::command(async)]
pub fn open_files(store: State<'_, MediaStore>, paths: Vec<String>) -> Vec<LibraryTrack> {
    let tracks: Vec<LibraryTrack> = paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| is_audio(p) && p.is_file())
        .map(|p| track_from_path(&p))
        .collect();
    // Persist so opened files show up under Local next time; a failed write
    // shouldn't stop playback, so the result is intentionally ignored.
    let _ = store.upsert_tracks(&tracks);
    tracks
}
