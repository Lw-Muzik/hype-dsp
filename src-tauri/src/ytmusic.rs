//! YouTube Music app state: the on-disk library cache and the download folder.
//!
//! Mirrors the cloud caches deliberately — same lazy-load, same write-then-rename,
//! same "serve the cache instantly, refresh behind it" contract the front end
//! already knows. What it does *not* mirror is `CloudMetaCache`: cloud tracks
//! list as bare filenames and need their tags range-read out of the container,
//! whereas YT Music hands us title/artist/album up front. So there's no tag
//! preload here, and no cover cache — just the listing.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use hm_ytmusic::{YtPlaylist, YtTrack};
use serde::{Deserialize, Serialize};

/// A whole library listing as cached on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedLibrary {
    #[serde(default)]
    pub playlists: Vec<YtPlaylist>,
    #[serde(default)]
    pub tracks: Vec<YtTrack>,
}

/// The last-listed library, so relaunching serves playlists instantly instead of
/// re-walking every playlist over the wire.
///
/// Loaded lazily for the same reason as `CloudListCache`: `setup()` runs before
/// first paint and this file can be large, so whoever needs it first pays the
/// parse, once.
pub struct YtLibraryCache {
    inner: OnceLock<Mutex<Option<CachedLibrary>>>,
    path: PathBuf,
}

impl YtLibraryCache {
    /// Builds the cache without touching the disk.
    pub fn new(path: PathBuf) -> Self {
        Self {
            inner: OnceLock::new(),
            path,
        }
    }

    fn cell(&self) -> &Mutex<Option<CachedLibrary>> {
        self.inner.get_or_init(|| {
            let loaded = std::fs::File::open(&self.path)
                .ok()
                .and_then(|f| serde_json::from_reader(std::io::BufReader::new(f)).ok());
            Mutex::new(loaded)
        })
    }

    /// Forces the one-time load now. Called from a background thread at startup.
    pub fn warm(&self) {
        let _ = self.cell();
    }

    pub fn get(&self) -> Option<CachedLibrary> {
        self.cell().lock().expect("yt cache poisoned").clone()
    }

    pub fn put(&self, lib: CachedLibrary) {
        {
            let mut slot = self.cell().lock().expect("yt cache poisoned");
            *slot = Some(lib);
        }
        self.write();
    }

    /// Drops the cached listing (on sign-out), on disk and in memory.
    pub fn clear(&self) {
        {
            let mut slot = self.cell().lock().expect("yt cache poisoned");
            *slot = None;
        }
        let _ = std::fs::remove_file(&self.path);
    }

    /// Write-then-rename so a crash mid-write can't leave a half-parsed file
    /// that reads as an empty library.
    fn write(&self) {
        let slot = self.cell().lock().expect("yt cache poisoned");
        let Some(lib) = slot.as_ref() else { return };
        let Ok(json) = serde_json::to_string(lib) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    /// Where downloads land. `None` means "use the default" — stored as absent
    /// rather than resolved, so a user who never chose a folder follows the OS
    /// music dir if it changes, instead of being pinned to wherever they first
    /// launched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    download_dir: Option<String>,
}

pub struct YtSettings {
    inner: Mutex<Settings>,
    path: PathBuf,
    /// The OS music dir, resolved at startup (Tauri's path API needs the app
    /// handle, which commands shouldn't have to thread through for this).
    default_dir: PathBuf,
}

impl YtSettings {
    pub fn load(path: PathBuf, default_dir: PathBuf) -> Self {
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(inner),
            path,
            default_dir,
        }
    }

    /// The download folder: whatever the user picked, else `<Music>/HypeMuzik`.
    pub fn download_dir(&self) -> PathBuf {
        self.inner
            .lock()
            .expect("yt settings poisoned")
            .download_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_dir.join("HypeMuzik"))
    }

    /// Sets the download folder. An empty string resets to the default.
    pub fn set_download_dir(&self, dir: Option<String>) {
        {
            let mut s = self.inner.lock().expect("yt settings poisoned");
            s.download_dir = dir.filter(|d| !d.trim().is_empty());
        }
        self.write();
    }

    fn write(&self) {
        let s = self.inner.lock().expect("yt settings poisoned");
        let Ok(json) = serde_json::to_string_pretty(&*s) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

/// Whether `path` sits inside `dir`, used to keep downloads where we expect.
pub fn is_within(dir: &Path, path: &Path) -> bool {
    match (dir.canonicalize(), path.canonicalize()) {
        (Ok(d), Ok(p)) => p.starts_with(d),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_music_subfolder() {
        let dir = std::env::temp_dir().join(format!("hm-yt-t{}", std::process::id()));
        let s = YtSettings::load(dir.join("s.json"), PathBuf::from("/Users/x/Music"));
        assert_eq!(s.download_dir(), PathBuf::from("/Users/x/Music/HypeMuzik"));
    }

    #[test]
    fn settings_round_trip_and_reset() {
        let base = std::env::temp_dir().join(format!("hm-yt-rt{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let file = base.join("settings.json");
        let s = YtSettings::load(file.clone(), PathBuf::from("/Music"));

        s.set_download_dir(Some("/tmp/picked".into()));
        assert_eq!(s.download_dir(), PathBuf::from("/tmp/picked"));

        // Reloading from disk sees the choice.
        let again = YtSettings::load(file.clone(), PathBuf::from("/Music"));
        assert_eq!(again.download_dir(), PathBuf::from("/tmp/picked"));

        // Blank resets to the default rather than storing an empty path.
        again.set_download_dir(Some("  ".into()));
        assert_eq!(again.download_dir(), PathBuf::from("/Music/HypeMuzik"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn library_cache_round_trips_and_clears() {
        let base = std::env::temp_dir().join(format!("hm-yt-lc{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("lib.json");

        let cache = YtLibraryCache::new(path.clone());
        assert!(cache.get().is_none(), "missing file reads as no cache");

        cache.put(CachedLibrary {
            playlists: vec![YtPlaylist {
                id: "PL1".into(),
                title: "Mix".into(),
                author: "me".into(),
                track_count: Some(1),
                thumbnail: None,
            }],
            tracks: vec![],
        });

        let reloaded = YtLibraryCache::new(path.clone());
        assert_eq!(reloaded.get().unwrap().playlists[0].title, "Mix");

        reloaded.clear();
        assert!(!path.exists());
        assert!(YtLibraryCache::new(path).get().is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn corrupt_cache_reads_as_empty_rather_than_panicking() {
        let base = std::env::temp_dir().join(format!("hm-yt-corrupt{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("lib.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(YtLibraryCache::new(path).get().is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn is_within_rejects_escapes() {
        let base = std::env::temp_dir().join(format!("hm-yt-w{}", std::process::id()));
        let inside = base.join("sub");
        std::fs::create_dir_all(&inside).unwrap();
        let file = inside.join("a.m4a");
        std::fs::write(&file, b"x").unwrap();

        assert!(is_within(&base, &file));
        assert!(!is_within(&inside, &std::env::temp_dir()));

        let _ = std::fs::remove_dir_all(&base);
    }
}
