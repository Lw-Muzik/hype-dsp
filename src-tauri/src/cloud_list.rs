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
use std::sync::Mutex;

use crate::cloud::CloudEntry;

/// Disk-backed map of account id → its flat account-wide audio listing.
pub struct CloudListCache {
    inner: Mutex<HashMap<String, Vec<CloudEntry>>>,
    path: PathBuf,
}

impl CloudListCache {
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

    /// The cached listing for `account_id`, if one was stored.
    pub fn get(&self, account_id: &str) -> Option<Vec<CloudEntry>> {
        self.inner.lock().ok()?.get(account_id).cloned()
    }

    /// Replace `account_id`'s listing and persist the whole map.
    pub fn put(&self, account_id: &str, entries: Vec<CloudEntry>) {
        let json = {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(account_id.to_string(), entries);
            serde_json::to_string(&*map).ok()
        };
        self.write(json);
    }

    /// Forget `account_id`'s listing (on disconnect) and persist.
    pub fn clear(&self, account_id: &str) {
        let json = {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
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
