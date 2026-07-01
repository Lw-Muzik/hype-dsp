//! Cloud track metadata cache.
//!
//! Cloud files (Google Drive / Dropbox) list only a name + handle, so their
//! tags are read by fetching the file's leading bytes and probing them (see
//! `hm_audio::fetch_stream_metadata`). That's a network round-trip per track, so
//! the results are cached on disk keyed by `"{account_id}:{file_id}"` and each
//! file is only downloaded once. Mirrors the mobile app's `CloudMetadataService`.
//!
//! **Text tags** (title/artist/album — tiny) live in an in-memory map persisted
//! to one small JSON file, written on a **debounced** background flush (not once
//! per insert). **Cover art** (a ~100 KB base64 `data:` URI each) is *not* kept
//! in memory or in that JSON — it is written to a **sharded on-disk store** (one
//! file per track) and read back lazily for the handful of on-screen rows. This
//! keeps a large cloud library from (a) holding hundreds of MB of covers in RAM
//! and (b) re-serializing the whole map to disk on every track during the
//! background preload (which was O(N²) writes).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    /// A copy carrying only the text tags — the cover is dropped so the bulk
    /// library hydrate never ships (nor holds) ~100 KB of base64 per track.
    pub fn tags_only(&self) -> Self {
        Self {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            cover: None,
        }
    }
}

/// Cover-less view used to load the persisted tags file. Deserializing into this
/// (rather than [`CloudTrackMeta`]) makes serde **skip** any `cover` field that a
/// legacy cache still has embedded, so the old big file streams in as tags only —
/// no cover strings are ever allocated. `serde_json::from_reader` streams the
/// file, so even a large legacy cache never lands in RAM whole.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

/// The shared, long-lived cache state (owned by the flusher thread too).
struct Inner {
    /// Tags only (cover always `None`), keyed by `"{account_id}:{file_id}"`.
    tags: Mutex<HashMap<String, CloudTrackMeta>>,
    /// Set on any tag insert; the flusher writes the map when it sees this.
    dirty: AtomicBool,
    /// Small JSON file holding just the tags map.
    tags_path: PathBuf,
    /// Directory of sharded cover files (one per track, name = hash of the key).
    covers_dir: PathBuf,
}

impl Inner {
    /// File holding one track's cover (`data:` URI). Keys can contain slashes
    /// (Dropbox paths), so the filename is a hash of the key, not the key itself.
    fn cover_path(&self, key: &str) -> PathBuf {
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        self.covers_dir.join(format!("{:016x}", h.finish()))
    }

    fn read_cover(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.cover_path(key)).ok()
    }

    fn write_cover(&self, key: &str, cover: &str) {
        // Best-effort: a failed cover write just means it's re-fetched next time.
        let _ = std::fs::write(self.cover_path(key), cover);
    }

    /// Serialize the tags map and atomically replace the file (write-then-rename).
    fn flush_to_disk(&self) {
        let json = {
            let map = self.tags.lock().unwrap_or_else(|e| e.into_inner());
            serde_json::to_string(&*map).ok()
        };
        if let Some(json) = json {
            let tmp = self.tags_path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.tags_path);
            }
        }
    }
}

/// Disk-backed cloud metadata cache: tags in RAM (debounced to one small JSON),
/// covers sharded on disk and read lazily.
pub struct CloudMetaCache {
    inner: Arc<Inner>,
}

impl CloudMetaCache {
    pub fn load(tags_path: PathBuf) -> Self {
        // Stream the persisted tags in, skipping any legacy embedded covers so a
        // large old cache never lands in RAM. (New files carry tags only.)
        let map: HashMap<String, CloudTrackMeta> = std::fs::File::open(&tags_path)
            .ok()
            .and_then(|f| {
                serde_json::from_reader::<_, HashMap<String, StoredTags>>(std::io::BufReader::new(f))
                    .ok()
            })
            .map(|raw| {
                raw.into_iter()
                    .map(|(k, t)| {
                        (
                            k,
                            CloudTrackMeta {
                                title: t.title,
                                artist: t.artist,
                                album: t.album,
                                cover: None,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let covers_dir = tags_path.with_file_name("cloud-covers");
        let _ = std::fs::create_dir_all(&covers_dir);

        let inner = Arc::new(Inner {
            tags: Mutex::new(map),
            dirty: AtomicBool::new(false),
            tags_path,
            covers_dir,
        });

        // Debounced background flush: write the (small) tags map at most once
        // every few seconds when it's been touched, instead of on every insert.
        // The cache lives for the whole session, so a detached thread is fine.
        {
            let inner = inner.clone();
            let _ = std::thread::Builder::new()
                .name("hm-cloud-meta-flush".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(2));
                    if inner.dirty.swap(false, Ordering::AcqRel) {
                        inner.flush_to_disk();
                    }
                });
        }

        Self { inner }
    }

    /// Full metadata (tags + cover) for one file, or `None` if never cached. The
    /// cover is read lazily from its shard, so this only touches disk for covers
    /// actually asked for (e.g. enriching the now-playing card).
    pub fn get(&self, key: &str) -> Option<CloudTrackMeta> {
        let mut meta = self.inner.tags.lock().ok()?.get(key).cloned()?;
        meta.cover = self.inner.read_cover(key);
        Some(meta)
    }

    /// Just the cached cover (a `data:` URI) for one file, if any. Lets the UI
    /// resolve a cover lazily for the handful of on-screen rows instead of
    /// hydrating every track's ~100 KB cover into memory up front.
    pub fn cover_for(&self, key: &str) -> Option<String> {
        self.inner.read_cover(key)
    }

    /// Every cached entry whose key starts with `"{prefix}:"`, re-keyed by the
    /// remaining file id — **tags only** (covers are never held in this map).
    /// Lets the front-end hydrate all of a provider's known titles/artists/albums
    /// in one cheap call on launch instead of one round-trip per track; covers
    /// resolve lazily per visible row.
    pub fn snapshot_tags_for(&self, prefix: &str) -> HashMap<String, CloudTrackMeta> {
        let want = format!("{prefix}:");
        let map = self.inner.tags.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .filter_map(|(k, v)| {
                k.strip_prefix(&want)
                    .map(|id| (id.to_string(), v.tags_only()))
            })
            .collect()
    }

    /// Cache one track's metadata: the cover (if any) goes to its on-disk shard,
    /// the tags into the in-memory map (marked dirty for the debounced flush).
    pub fn put(&self, key: String, meta: CloudTrackMeta) {
        if let Some(cover) = &meta.cover {
            self.inner.write_cover(&key, cover);
        }
        {
            let mut map = self.inner.tags.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(key, meta.tags_only());
        }
        self.inner.dirty.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_in(dir: &std::path::Path) -> CloudMetaCache {
        CloudMetaCache::load(dir.join("hm_cloud_meta.json"))
    }

    #[test]
    fn stores_tags_in_map_and_cover_on_disk() {
        let tmp = std::env::temp_dir().join(format!("hm_meta_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let cache = cache_in(&tmp);

        cache.put(
            "acct:file1".into(),
            CloudTrackMeta {
                title: Some("Song".into()),
                artist: Some("Artist".into()),
                album: None,
                cover: Some("data:image/png;base64,AAAA".into()),
            },
        );

        // Tags come back; the cover is resolved lazily from its shard.
        let got = cache.get("acct:file1").expect("cached");
        assert_eq!(got.title.as_deref(), Some("Song"));
        assert_eq!(got.cover.as_deref(), Some("data:image/png;base64,AAAA"));
        assert_eq!(cache.cover_for("acct:file1").as_deref(), Some("data:image/png;base64,AAAA"));

        // The bulk tags snapshot never carries covers.
        let snap = cache.snapshot_tags_for("acct");
        assert_eq!(snap.get("file1").and_then(|m| m.title.clone()).as_deref(), Some("Song"));
        assert!(snap.get("file1").unwrap().cover.is_none(), "snapshot is cover-free");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unknown_key_is_none() {
        let tmp = std::env::temp_dir().join(format!("hm_meta_test2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let cache = cache_in(&tmp);
        assert!(cache.get("acct:missing").is_none());
        assert!(cache.cover_for("acct:missing").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
