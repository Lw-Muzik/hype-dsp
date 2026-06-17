//! SQLite-backed persistence for EQ presets.
//!
//! Built-in genre presets are (re)seeded on open; custom presets are full CRUD.
//! The `rusqlite` `Connection` is `!Sync`, so it lives behind a `Mutex` — all
//! access is off the audio thread, so locking is fine.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::HmError;
use crate::presets::builtins;
use crate::{EqPreset, BAND_COUNT};

/// EQ preset store backed by SQLite.
pub struct PresetStore {
    conn: Mutex<Connection>,
}

impl PresetStore {
    /// Open (creating if needed) the store at `path`, initialize the schema,
    /// and seed the built-in presets.
    pub fn open(path: &Path) -> Result<Self, HmError> {
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    /// In-memory store, for tests.
    pub fn open_in_memory() -> Result<Self, HmError> {
        let conn = Connection::open_in_memory()?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self, HmError> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS eq_presets (
                id       TEXT PRIMARY KEY,
                name     TEXT NOT NULL,
                builtin  INTEGER NOT NULL,
                bands    TEXT NOT NULL,
                pre_gain REAL NOT NULL,
                ord      INTEGER NOT NULL DEFAULT 1000
            )",
            [],
        )?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.seed_builtins()?;
        Ok(store)
    }

    fn seed_builtins(&self) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("preset store poisoned");
        for (ord, preset) in builtins().into_iter().enumerate() {
            let bands = serde_json::to_string(&preset.bands.to_vec())?;
            // REPLACE keeps built-ins current across versions; custom ids never
            // collide (they are `custom:*`).
            conn.execute(
                "INSERT OR REPLACE INTO eq_presets (id, name, builtin, bands, pre_gain, ord)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5)",
                rusqlite::params![preset.id, preset.name, bands, preset.pre_gain, ord as i64],
            )?;
        }
        Ok(())
    }

    /// All presets, built-ins first (in shipped order), then custom by name.
    pub fn list(&self) -> Result<Vec<EqPreset>, HmError> {
        let conn = self.conn.lock().expect("preset store poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, builtin, bands, pre_gain FROM eq_presets ORDER BY ord, name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(to_preset(row?)?);
        }
        Ok(out)
    }

    /// Fetch one preset by id.
    pub fn get(&self, id: &str) -> Result<EqPreset, HmError> {
        let conn = self.conn.lock().expect("preset store poisoned");
        let mut stmt = conn
            .prepare("SELECT id, name, builtin, bands, pre_gain FROM eq_presets WHERE id = ?1")?;
        let mut rows = stmt.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        match rows.next() {
            Some(row) => to_preset(row?),
            None => Err(HmError::NotFound(format!("preset {id}"))),
        }
    }

    /// Save a new custom preset; returns it with its generated id.
    pub fn save_custom(
        &self,
        name: &str,
        bands: [f32; BAND_COUNT],
        pre_gain: f32,
    ) -> Result<EqPreset, HmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HmError::Invalid("preset name is empty".into()));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let preset = EqPreset {
            id: format!("custom:{nanos}"),
            name: name.to_string(),
            builtin: false,
            bands,
            pre_gain,
        };
        let conn = self.conn.lock().expect("preset store poisoned");
        conn.execute(
            "INSERT INTO eq_presets (id, name, builtin, bands, pre_gain, ord)
             VALUES (?1, ?2, 0, ?3, ?4, 1000)",
            rusqlite::params![
                preset.id,
                preset.name,
                serde_json::to_string(&preset.bands.to_vec())?,
                preset.pre_gain
            ],
        )?;
        Ok(preset)
    }

    /// Update an existing custom preset (built-ins are immutable).
    pub fn update(&self, preset: &EqPreset) -> Result<(), HmError> {
        if preset.builtin || preset.id.starts_with("builtin:") {
            return Err(HmError::Invalid("built-in presets cannot be edited".into()));
        }
        let conn = self.conn.lock().expect("preset store poisoned");
        let changed = conn.execute(
            "UPDATE eq_presets SET name = ?2, bands = ?3, pre_gain = ?4
             WHERE id = ?1 AND builtin = 0",
            rusqlite::params![
                preset.id,
                preset.name,
                serde_json::to_string(&preset.bands.to_vec())?,
                preset.pre_gain
            ],
        )?;
        if changed == 0 {
            return Err(HmError::NotFound(format!("custom preset {}", preset.id)));
        }
        Ok(())
    }

    /// Delete a custom preset (built-ins cannot be deleted).
    pub fn delete(&self, id: &str) -> Result<(), HmError> {
        if id.starts_with("builtin:") {
            return Err(HmError::Invalid(
                "built-in presets cannot be deleted".into(),
            ));
        }
        let conn = self.conn.lock().expect("preset store poisoned");
        let changed = conn.execute("DELETE FROM eq_presets WHERE id = ?1 AND builtin = 0", [id])?;
        if changed == 0 {
            return Err(HmError::NotFound(format!("custom preset {id}")));
        }
        Ok(())
    }
}

fn to_preset(row: (String, String, i64, String, f64)) -> Result<EqPreset, HmError> {
    let (id, name, builtin, bands_json, pre_gain) = row;
    let values: Vec<f32> = serde_json::from_str(&bands_json)?;
    let bands: [f32; BAND_COUNT] = values
        .try_into()
        .map_err(|_| HmError::Invalid(format!("preset {id} has the wrong band count")))?;
    Ok(EqPreset {
        id,
        name,
        builtin: builtin != 0,
        bands,
        pre_gain: pre_gain as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_builtins_on_open() {
        let store = PresetStore::open_in_memory().unwrap();
        let all = store.list().unwrap();
        assert!(all.len() >= 10, "expected the built-in genre presets");
        assert_eq!(all[0].id, "builtin:flat", "Flat should sort first");
        assert!(all
            .iter()
            .all(|p| !p.builtin || p.id.starts_with("builtin:")));
    }

    #[test]
    fn custom_preset_crud_roundtrip() {
        let store = PresetStore::open_in_memory().unwrap();
        let mut bands = [0.0f32; BAND_COUNT];
        bands[10] = 5.0;

        let saved = store.save_custom("My Preset", bands, -2.0).unwrap();
        assert!(saved.id.starts_with("custom:"));
        assert!(!saved.builtin);

        let fetched = store.get(&saved.id).unwrap();
        assert_eq!(fetched.bands[10], 5.0);
        assert_eq!(fetched.pre_gain, -2.0);

        let mut edited = fetched.clone();
        edited.name = "Renamed".into();
        edited.bands[0] = -3.0;
        store.update(&edited).unwrap();
        assert_eq!(store.get(&saved.id).unwrap().name, "Renamed");
        assert_eq!(store.get(&saved.id).unwrap().bands[0], -3.0);

        store.delete(&saved.id).unwrap();
        assert!(store.get(&saved.id).is_err());
    }

    #[test]
    fn builtins_cannot_be_deleted_or_edited() {
        let store = PresetStore::open_in_memory().unwrap();
        assert!(store.delete("builtin:flat").is_err());
        let flat = store.get("builtin:flat").unwrap();
        assert!(store.update(&flat).is_err());
    }
}
