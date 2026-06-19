//! SQLite-backed persistence for the local music library and playlists.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::HmError;
use crate::{LibraryTrack, Playlist, RadioStation};

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Library + playlist store.
pub struct MediaStore {
    conn: Mutex<Connection>,
}

impl MediaStore {
    pub fn open(path: &Path) -> Result<Self, HmError> {
        Self::from_conn(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, HmError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self, HmError> {
        // WAL + relaxed sync make bulk imports (tens of thousands of tracks)
        // dramatically faster — one fsync per checkpoint instead of per row —
        // while staying crash-safe for a media catalog.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                path          TEXT PRIMARY KEY,
                title         TEXT NOT NULL,
                artist        TEXT,
                album         TEXT,
                genre         TEXT,
                duration_secs REAL,
                added_at      INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlists (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlist_items (
                playlist_id TEXT NOT NULL,
                position    INTEGER NOT NULL,
                track_path  TEXT NOT NULL,
                PRIMARY KEY (playlist_id, position)
            );
            CREATE TABLE IF NOT EXISTS radio_favorites (
                id       TEXT PRIMARY KEY,
                name     TEXT NOT NULL,
                url      TEXT NOT NULL,
                genre    TEXT,
                country  TEXT,
                favicon  TEXT,
                added_at INTEGER NOT NULL
            );",
        )?;
        // Migration: add `genre` to libraries created before category filtering.
        // (SQLite has no "ADD COLUMN IF NOT EXISTS"; ignore the duplicate-column error.)
        let _ = conn.execute("ALTER TABLE tracks ADD COLUMN genre TEXT", []);
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert or update a library track (preserves `added_at` on update).
    pub fn upsert_track(&self, track: &LibraryTrack) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute(
            "INSERT INTO tracks (path, title, artist, album, genre, duration_secs, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
               title = excluded.title,
               artist = excluded.artist,
               album = excluded.album,
               genre = excluded.genre,
               duration_secs = excluded.duration_secs",
            rusqlite::params![
                track.path,
                track.title,
                track.artist,
                track.album,
                track.genre,
                track.duration_secs,
                now_millis()
            ],
        )?;
        Ok(())
    }

    /// Insert/update many tracks in a single transaction — the fast path for a
    /// library scan (one fsync for the whole batch instead of one per track).
    pub fn upsert_tracks(&self, tracks: &[LibraryTrack]) -> Result<(), HmError> {
        if tracks.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().expect("media store poisoned");
        let tx = conn.transaction()?;
        let now = now_millis();
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO tracks (path, title, artist, album, genre, duration_secs, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(path) DO UPDATE SET
                   title = excluded.title,
                   artist = excluded.artist,
                   album = excluded.album,
                   genre = excluded.genre,
                   duration_secs = excluded.duration_secs",
            )?;
            for t in tracks {
                stmt.execute(rusqlite::params![
                    t.path,
                    t.title,
                    t.artist,
                    t.album,
                    t.genre,
                    t.duration_secs,
                    now
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_tracks(&self) -> Result<Vec<LibraryTrack>, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt = conn.prepare(
            "SELECT path, title, artist, album, genre, duration_secs FROM tracks
             ORDER BY title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], row_to_track)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn remove_track(&self, path: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute("DELETE FROM tracks WHERE path = ?1", [path])?;
        conn.execute("DELETE FROM playlist_items WHERE track_path = ?1", [path])?;
        Ok(())
    }

    pub fn list_playlists(&self) -> Result<Vec<Playlist>, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt =
            conn.prepare("SELECT id, name FROM playlists ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_playlist(&self, name: &str) -> Result<Playlist, HmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HmError::Invalid("playlist name is empty".into()));
        }
        let id = format!("pl:{}", now_millis());
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute(
            "INSERT INTO playlists (id, name, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, name, now_millis()],
        )?;
        Ok(Playlist {
            id,
            name: name.to_string(),
        })
    }

    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let changed = conn.execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            rusqlite::params![id, name],
        )?;
        if changed == 0 {
            return Err(HmError::NotFound(format!("playlist {id}")));
        }
        Ok(())
    }

    pub fn delete_playlist(&self, id: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
        conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Tracks in a playlist, in order.
    pub fn playlist_tracks(&self, id: &str) -> Result<Vec<LibraryTrack>, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt = conn.prepare(
            "SELECT t.path, t.title, t.artist, t.album, t.genre, t.duration_secs
             FROM playlist_items pi JOIN tracks t ON t.path = pi.track_path
             WHERE pi.playlist_id = ?1 ORDER BY pi.position",
        )?;
        let rows = stmt.query_map([id], row_to_track)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Append a track to a playlist (no-op if already present).
    pub fn add_to_playlist(&self, playlist_id: &str, track_path: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlist_items WHERE playlist_id=?1 AND track_path=?2)",
            rusqlite::params![playlist_id, track_path],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(());
        }
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id = ?1",
            [playlist_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO playlist_items (playlist_id, position, track_path) VALUES (?1, ?2, ?3)",
            rusqlite::params![playlist_id, next, track_path],
        )?;
        Ok(())
    }

    pub fn remove_from_playlist(&self, playlist_id: &str, track_path: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1 AND track_path = ?2",
            rusqlite::params![playlist_id, track_path],
        )?;
        Ok(())
    }

    /// Rewrite a playlist's order from the given track paths.
    pub fn reorder_playlist(&self, playlist_id: &str, paths: &[String]) -> Result<(), HmError> {
        let mut conn = self.conn.lock().expect("media store poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (pos, path) in paths.iter().enumerate() {
            tx.execute(
                "INSERT INTO playlist_items (playlist_id, position, track_path) VALUES (?1, ?2, ?3)",
                rusqlite::params![playlist_id, pos as i64, path],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl MediaStore {
    /// Favorited radio stations.
    pub fn list_favorites(&self) -> Result<Vec<RadioStation>, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, url, genre, country, favicon FROM radio_favorites
             ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(RadioStation {
                id: r.get(0)?,
                name: r.get(1)?,
                url: r.get(2)?,
                genre: r.get(3)?,
                country: r.get(4)?,
                favicon: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_favorite(&self, s: &RadioStation) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO radio_favorites (id, name, url, genre, country, favicon, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![s.id, s.name, s.url, s.genre, s.country, s.favicon, now_millis()],
        )?;
        Ok(())
    }

    pub fn remove_favorite(&self, id: &str) -> Result<(), HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        conn.execute("DELETE FROM radio_favorites WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_track(r: &rusqlite::Row) -> rusqlite::Result<LibraryTrack> {
    Ok(LibraryTrack {
        path: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
        album: r.get(3)?,
        genre: r.get(4)?,
        duration_secs: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, title: &str) -> LibraryTrack {
        LibraryTrack {
            path: path.into(),
            title: title.into(),
            artist: Some("Artist".into()),
            album: None,
            genre: None,
            duration_secs: Some(123.0),
        }
    }

    #[test]
    fn tracks_upsert_and_list() {
        let store = MediaStore::open_in_memory().unwrap();
        store.upsert_track(&track("/a.flac", "Alpha")).unwrap();
        store.upsert_track(&track("/b.mp3", "Bravo")).unwrap();
        store.upsert_track(&track("/a.flac", "Alpha v2")).unwrap(); // update
        let tracks = store.list_tracks().unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().any(|t| t.title == "Alpha v2"));
    }

    #[test]
    fn playlist_lifecycle() {
        let store = MediaStore::open_in_memory().unwrap();
        store.upsert_track(&track("/a.flac", "Alpha")).unwrap();
        store.upsert_track(&track("/b.mp3", "Bravo")).unwrap();

        let pl = store.create_playlist("Favourites").unwrap();
        store.add_to_playlist(&pl.id, "/a.flac").unwrap();
        store.add_to_playlist(&pl.id, "/b.mp3").unwrap();
        store.add_to_playlist(&pl.id, "/a.flac").unwrap(); // dedup
        assert_eq!(store.playlist_tracks(&pl.id).unwrap().len(), 2);

        store
            .reorder_playlist(&pl.id, &["/b.mp3".into(), "/a.flac".into()])
            .unwrap();
        assert_eq!(store.playlist_tracks(&pl.id).unwrap()[0].path, "/b.mp3");

        store.remove_from_playlist(&pl.id, "/b.mp3").unwrap();
        assert_eq!(store.playlist_tracks(&pl.id).unwrap().len(), 1);

        store.delete_playlist(&pl.id).unwrap();
        assert!(store.list_playlists().unwrap().is_empty());
    }
}
