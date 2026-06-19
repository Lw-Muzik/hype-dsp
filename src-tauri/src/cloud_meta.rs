//! Cloud track metadata cache.
//!
//! Cloud files (Google Drive / Dropbox) list only a name + handle, so their
//! tags are read by fetching the file's leading bytes and probing them (see
//! `hm_audio::fetch_stream_metadata`). That's a network round-trip per track, so
//! the results — title/artist/album + cover — are cached on disk keyed by
//! `"{provider}:{file_id}"` and each file is only downloaded once. Mirrors the
//! mobile app's `CloudMetadataService` (background preload + persistent cache).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Extracted metadata for one cloud track. `cover` is a `data:` URI when the
/// file had embedded front-cover art.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudTrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cover: Option<String>,
}

impl CloudTrackMeta {
    /// Whether anything worth caching/showing was found.
    pub fn is_useful(&self) -> bool {
        self.title.is_some()
            || self.artist.is_some()
            || self.album.is_some()
            || self.cover.is_some()
    }
}

/// Disk-backed map of `"{provider}:{file_id}"` → [`CloudTrackMeta`].
pub struct CloudMetaCache {
    inner: Mutex<HashMap<String, CloudTrackMeta>>,
    path: PathBuf,
}

impl CloudMetaCache {
    pub fn load(path: PathBuf) -> Self {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(map),
            path,
        }
    }

    pub fn get(&self, key: &str) -> Option<CloudTrackMeta> {
        self.inner.lock().ok()?.get(key).cloned()
    }

    /// Insert `meta` and persist the whole map (write-then-rename, like the
    /// token store).
    pub fn put(&self, key: String, meta: CloudTrackMeta) {
        let json = {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(key, meta);
            serde_json::to_string(&*map).ok()
        };
        if let Some(json) = json {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}
