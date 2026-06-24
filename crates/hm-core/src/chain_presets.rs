//! JSON-file-backed store for whole-chain DSP presets.
//!
//! Each [`ChainPreset`] captures a complete [`EngineState`] snapshot under a
//! human-readable name so the user can save their current sound, recall it
//! later, and share it with others via export/import.
//!
//! The store is a plain JSON array written to a single file.  The entire list
//! is small (tens of items at most), so a whole-file rewrite on every mutation
//! is simpler and safer than in-place patching.  Writes go through a
//! write-then-rename (atomic replace) pattern to guard against corruption on
//! crash.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::HmError;
use crate::EngineState;

// ── Data type ────────────────────────────────────────────────────────────────

/// A named snapshot of the entire DSP enhancement chain.
///
/// Mirrored by `ChainPreset` in `src/lib/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainPreset {
    /// Stable opaque identifier (millis-based, unique within this store).
    pub id: String,
    /// User-visible name chosen at save time.
    pub name: String,
    /// Full engine state snapshot.
    pub state: EngineState,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// File-backed store for [`ChainPreset`]s.
///
/// The path is supplied at construction; no file I/O happens in [`open`] — the
/// file is read on every [`list`] call and written on every mutation so the
/// caller never has to worry about flushing.
pub struct ChainPresetStore {
    path: PathBuf,
}

impl ChainPresetStore {
    /// Create a store that reads from and writes to `path`.
    ///
    /// The file does not need to exist yet; it is created on first write.
    pub fn open(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    /// Read the full preset list from disk.
    ///
    /// Returns an empty list when the file is absent or empty — this is the
    /// normal state for a fresh install, not an error.
    pub fn list(&self) -> Result<Vec<ChainPreset>, HmError> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(vec![]),
            Ok(s) => {
                let presets: Vec<ChainPreset> = serde_json::from_str(&s)?;
                Ok(presets)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(HmError::Storage(e.to_string())),
        }
    }

    /// Save the current engine state under `name`, appending it to the list.
    ///
    /// Returns the newly created preset (with its generated id).
    pub fn save(&self, name: &str, state: EngineState) -> Result<ChainPreset, HmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HmError::Invalid("preset name is empty".into()));
        }
        let mut list = self.list()?;
        let id = unique_id(list.len());
        let preset = ChainPreset {
            id,
            name: name.to_string(),
            state,
        };
        list.push(preset.clone());
        self.write(&list)?;
        Ok(preset)
    }

    /// Delete the preset with the given `id`.
    ///
    /// Returns [`HmError::NotFound`] when no preset with that id exists.
    pub fn delete(&self, id: &str) -> Result<(), HmError> {
        let mut list = self.list()?;
        let before = list.len();
        list.retain(|p| p.id != id);
        if list.len() == before {
            return Err(HmError::NotFound(format!("chain preset {id}")));
        }
        self.write(&list)
    }

    /// Add an imported preset, assigning a **fresh** id to avoid collisions.
    ///
    /// The incoming `preset.id` is ignored; a new unique id is generated from
    /// the current time and list length.  Returns the stored preset.
    pub fn upsert_imported(&self, preset: ChainPreset) -> Result<ChainPreset, HmError> {
        let mut list = self.list()?;
        let id = unique_id(list.len());
        let stored = ChainPreset {
            id,
            name: preset.name,
            state: preset.state,
        };
        list.push(stored.clone());
        self.write(&list)?;
        Ok(stored)
    }

    // ── private ───────────────────────────────────────────────────────────────

    /// Atomically replace the store file with the serialized `list`.
    ///
    /// Writes to a `.tmp` sibling first, then renames — so a crash mid-write
    /// never leaves a partial file behind.
    fn write(&self, list: &[ChainPreset]) -> Result<(), HmError> {
        // Ensure the parent directory exists.
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HmError::Storage(format!("create dir: {e}")))?;
        }

        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(list)?;
        std::fs::write(&tmp, &json)
            .map_err(|e| HmError::Storage(format!("write tmp: {e}")))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| HmError::Storage(format!("rename: {e}")))?;
        Ok(())
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Generate a unique id from the current millisecond timestamp and a
/// list-length disambiguator so that rapid back-to-back saves within the same
/// millisecond still produce distinct ids.
fn unique_id(list_len: usize) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}{}", millis, list_len)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a throwaway temp path for one test.  Include the test name so
    /// parallel test runs never collide.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hm_chain_presets_{}.json", name))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    }

    #[test]
    fn save_then_list_roundtrips() {
        let path = temp_path("roundtrip");
        cleanup(&path);
        let store = ChainPresetStore::open(&path);

        let state_a = EngineState::default();
        let state_b = EngineState {
            master_volume: 1.5,
            ..EngineState::default()
        };

        store.save("Warm", state_a.clone()).unwrap();
        store.save("Punch", state_b.clone()).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Warm");
        assert_eq!(list[1].name, "Punch");
        assert_eq!(list[0].state.master_volume, state_a.master_volume);
        assert_eq!(list[1].state.master_volume, state_b.master_volume);

        cleanup(&path);
    }

    #[test]
    fn delete_removes() {
        let path = temp_path("delete");
        cleanup(&path);
        let store = ChainPresetStore::open(&path);

        let p1 = store.save("Keep", EngineState::default()).unwrap();
        let p2 = store.save("Remove", EngineState::default()).unwrap();

        store.delete(&p2.id).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, p1.id);
        assert_eq!(list[0].name, "Keep");

        cleanup(&path);
    }

    #[test]
    fn list_empty_when_absent() {
        let path = temp_path("absent");
        cleanup(&path);
        // No file created — list() must succeed and return [].
        let store = ChainPresetStore::open(&path);
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn upsert_imported_assigns_fresh_id() {
        let path = temp_path("upsert");
        cleanup(&path);
        let store = ChainPresetStore::open(&path);

        // Save one preset so we have a known id.
        let existing = store.save("Existing", EngineState::default()).unwrap();

        // Craft an import whose id matches the existing one.
        let import = ChainPreset {
            id: existing.id.clone(), // intentional collision
            name: "Imported".to_string(),
            state: EngineState::default(),
        };

        let stored = store.upsert_imported(import).unwrap();

        // Must have been assigned a different id.
        assert_ne!(stored.id, existing.id);

        // Both must be in the list with distinct ids.
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);

        cleanup(&path);
    }

    #[test]
    fn persists_across_reopen() {
        let path = temp_path("reopen");
        cleanup(&path);

        {
            let store = ChainPresetStore::open(&path);
            store.save("Persistent", EngineState::default()).unwrap();
        } // store dropped here

        // Re-open from the same path.
        let store2 = ChainPresetStore::open(&path);
        let list = store2.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Persistent");

        cleanup(&path);
    }

    #[test]
    fn unique_ids_on_rapid_save() {
        let path = temp_path("rapid");
        cleanup(&path);
        let store = ChainPresetStore::open(&path);

        store.save("A", EngineState::default()).unwrap();
        store.save("B", EngineState::default()).unwrap();
        store.save("C", EngineState::default()).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);

        let mut ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "all three ids must be distinct");

        cleanup(&path);
    }
}
