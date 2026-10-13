//! The playback session file: what to bring back on the next launch (queue,
//! position, shuffle/repeat). The frontend owns the format
//! (`src/features/session/playbackSession.ts`); this only stores the text.
//!
//! A file in the app data folder rather than webview storage: a 500-track
//! queue doesn't belong in localStorage, which is also wiped with the
//! webview's data.

use std::path::{Path, PathBuf};

use hm_core::IpcError;
use tauri::{AppHandle, Manager};

const FILE_NAME: &str = "playback-session.json";
/// Far above a windowed 500-track session; anything larger is not ours.
const MAX_BYTES: usize = 8 * 1024 * 1024;

fn session_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FILE_NAME))
}

/// The saved session, if there is one.
#[tauri::command(async)]
pub fn session_load(app: AppHandle) -> Option<String> {
    crate::blocking::io(|| {
        let path = session_path(&app)?;
        read(&path)
    })
}

/// Save the session (`None` clears it: nothing to come back to).
#[tauri::command(async)]
pub fn session_save(app: AppHandle, json: Option<String>) -> Result<(), IpcError> {
    crate::blocking::io(|| {
        let path = session_path(&app).ok_or_else(|| IpcError::new("session", "no app data folder"))?;
        write(&path, json.as_deref()).map_err(|e| IpcError::new("session", e))
    })
}

fn read(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    (text.len() <= MAX_BYTES).then_some(text)
}

/// Write via a temp file and rename, so a crash mid-write never leaves half a
/// session behind (the old one survives instead).
fn write(path: &Path, json: Option<&str>) -> Result<(), String> {
    let Some(json) = json else {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    };
    if json.len() > MAX_BYTES {
        return Err(format!("session too large ({} bytes)", json.len()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hm_session_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn round_trips_and_clears() {
        let path = temp("rt");
        assert_eq!(read(&path), None);
        write(&path, Some(r#"{"v":1}"#)).unwrap();
        assert_eq!(read(&path).as_deref(), Some(r#"{"v":1}"#));
        // Overwrite leaves no temp file behind.
        write(&path, Some(r#"{"v":1,"x":2}"#)).unwrap();
        assert!(!path.with_extension("json.part").exists());
        assert_eq!(read(&path).as_deref(), Some(r#"{"v":1,"x":2}"#));
        write(&path, None).unwrap();
        assert_eq!(read(&path), None);
        // Clearing what isn't there is fine.
        write(&path, None).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refuses_oversized_sessions() {
        let path = temp("big");
        assert!(write(&path, Some(&"x".repeat(MAX_BYTES + 1))).is_err());
        assert_eq!(read(&path), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
