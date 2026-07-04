//! Cloud audio-listing cache.
//!
//! `cloud_all_audio` walks a connected account for every audio file — a full
//! network listing of the account. The front-end library store is in-memory,
//! so without a cache that walk re-runs on every app launch and the library
//! appears to "reload as if you'd just connected". To make relaunches instant
//! the flat listing is cached on disk per account (write-then-rename, like
//! [`crate::cloud_meta::CloudMetaCache`]) and served immediately, while a
//! background refresh keeps it current. The entry is cleared when an account
//! disconnects so a freshly connected account can't show another's stale files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::cloud::CloudEntry;

/// Disk-backed map of account id → its flat account-wide audio listing.
///
/// The on-disk file can be MBs for a big account, so it is **not** parsed at
/// construction (which happens in `setup()`, before first paint): the map is
/// loaded on first touch through [`Self::map`], and a background warmer thread
/// (spawned in `lib.rs`) funnels through the same `OnceLock` so whoever gets
/// there first pays the parse exactly once.
pub struct CloudListCache {
    inner: OnceLock<Mutex<HashMap<String, Vec<CloudEntry>>>>,
    path: PathBuf,
}

impl CloudListCache {
    /// Point the cache at `path` without reading it — cheap enough for the
    /// startup path. The actual load happens lazily (see [`Self::warm`]).
    pub fn new(path: PathBuf) -> Self {
        Self {
            inner: OnceLock::new(),
            path,
        }
    }

    /// Get-or-load funnel: parse the on-disk file the first time anything
    /// (warmer thread or a cloud command) needs the map.
    fn map(&self) -> &Mutex<HashMap<String, Vec<CloudEntry>>> {
        self.inner.get_or_init(|| {
            Mutex::new(
                std::fs::read_to_string(&self.path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default(),
            )
        })
    }

    /// Load the on-disk cache now if it hasn't been touched yet — called from a
    /// background thread right after startup so the first `cloud_all_audio` is
    /// (usually) a warm hit without the main thread ever paying the parse.
    pub fn warm(&self) {
        let _ = self.map();
    }

    /// The cached listing for `account_id`, if one was stored.
    pub fn get(&self, account_id: &str) -> Option<Vec<CloudEntry>> {
        self.map().lock().ok()?.get(account_id).cloned()
    }

    /// Replace `account_id`'s listing and persist the whole map.
    pub fn put(&self, account_id: &str, entries: Vec<CloudEntry>) {
        let json = {
            let mut map = self.map().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(account_id.to_string(), entries);
            serde_json::to_string(&*map).ok()
        };
        self.write(json);
    }

    /// Forget `account_id`'s listing (on disconnect) and persist.
    pub fn clear(&self, account_id: &str) {
        let json = {
            let mut map = self.map().lock().unwrap_or_else(|e| e.into_inner());
            map.remove(account_id);
            serde_json::to_string(&*map).ok()
        };
        self.write(json);
    }

    fn write(&self, json: Option<String>) {
        if let Some(json) = json {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}
