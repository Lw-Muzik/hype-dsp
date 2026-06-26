//! SQLite-backed persistence for the local music library and playlists.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::HmError;
use crate::{LibraryPage, LibraryTrack, Playlist, RadioStation};

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
            );
            -- Backs the `ORDER BY title COLLATE NOCASE` used by listing/paging so
            -- a huge library (100k–1M tracks) pages by index seek instead of a
            -- full scan + sort on every read. Created once; backfills old DBs.
            CREATE INDEX IF NOT EXISTS idx_tracks_title_nocase
                ON tracks(title COLLATE NOCASE);",
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

    /// Total number of library tracks — cheap, lets the UI show a load progress
    /// fraction ("12,000 / 234,000") rather than an open-ended spinner.
    pub fn count_tracks(&self) -> Result<i64, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        Ok(conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?)
    }

    /// One ordered page of **currently reachable** library tracks (same
    /// `ORDER BY title` as `list_tracks`). The UI pages these in incrementally so
    /// it never parses the whole library in one blocking task; the
    /// `idx_tracks_title_nocase` index keeps each page an index seek even at 1M
    /// tracks. A negative `offset`/`limit` is clamped to 0.
    ///
    /// Rows whose file is not reachable right now (e.g. an unplugged external
    /// drive) are filtered out so they don't linger in the library — but they
    /// stay in the DB, so reconnecting the drive brings them back with no
    /// re-scan. `LibraryPage::scanned` reports how many DB rows were read before
    /// filtering, so the caller advances its offset correctly even when a page
    /// is partly (or wholly) hidden.
    pub fn list_tracks_page(&self, offset: i64, limit: i64) -> Result<LibraryPage, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt = conn.prepare(
            "SELECT path, title, artist, album, genre, duration_secs FROM tracks
             ORDER BY title COLLATE NOCASE
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit.max(0), offset.max(0)], row_to_track)?;
        let mut dir_cache: HashMap<PathBuf, bool> = HashMap::new();
        let mut tracks = Vec::new();
        let mut scanned: i64 = 0;
        for row in rows {
            let t = row?;
            scanned += 1;
            if track_available(&t.path, &mut dir_cache) {
                tracks.push(t);
            }
        }
        Ok(LibraryPage { tracks, scanned })
    }

    /// How many library tracks are currently reachable (their file's directory
    /// exists). Cheap (one `path` scan + a cached `stat` per distinct directory),
    /// so the UI can probe it on focus and only reload when availability
    /// actually changed — e.g. a drive was plugged in or ejected.
    pub fn count_available_tracks(&self) -> Result<i64, HmError> {
        let conn = self.conn.lock().expect("media store poisoned");
        let mut stmt = conn.prepare("SELECT path FROM tracks")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut dir_cache: HashMap<PathBuf, bool> = HashMap::new();
        let mut available: i64 = 0;
        for row in rows {
            if track_available(&row?, &mut dir_cache) {
                available += 1;
            }
        }
        Ok(available)
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

/// Whether a track's file is currently reachable, judged by its parent
/// directory existing. The `dir_cache` collapses a whole disconnected drive to a
/// handful of `stat`s — every file under a missing directory shares the cached
/// miss — instead of one syscall per track. Checking the *directory* (not the
/// file) is both cheaper (cache hits across siblings) and the right granularity
/// for the unplugged-drive case, where the entire mount point disappears.
fn track_available(path: &str, dir_cache: &mut HashMap<PathBuf, bool>) -> bool {
    match Path::new(path).parent() {
        Some(parent) => *dir_cache
            .entry(parent.to_path_buf())
            .or_insert_with(|| parent.is_dir()),
        // A bare filename has no directory to judge — don't hide it.
        None => true,
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
    fn count_tracks_reports_total() {
        let store = MediaStore::open_in_memory().unwrap();
        assert_eq!(store.count_tracks().unwrap(), 0);
        store.upsert_track(&track("/a.flac", "Alpha")).unwrap();
        store.upsert_track(&track("/b.mp3", "Bravo")).unwrap();
        store.upsert_track(&track("/a.flac", "Alpha v2")).unwrap(); // update, not new
        assert_eq!(store.count_tracks().unwrap(), 2);
    }

    #[test]
    fn list_tracks_page_is_ordered_and_bounded() {
        let store = MediaStore::open_in_memory().unwrap();
        // Insert out of order; listing/paging must come back title-sorted.
        for (p, t) in [
            ("/3.mp3", "Charlie"),
            ("/1.mp3", "alpha"), // lowercase: COLLATE NOCASE sorts it first
            ("/4.mp3", "Delta"),
            ("/2.mp3", "Bravo"),
            ("/5.mp3", "Echo"),
        ] {
            store.upsert_track(&track(p, t)).unwrap();
        }

        // These test paths live at root (`/…`), whose parent `/` exists, so the
        // availability filter keeps them all — see `page_hides_unreachable_files`
        // for the filtering behaviour itself.

        // Page 1 of size 2 → first two titles in NOCASE order.
        let p0 = store.list_tracks_page(0, 2).unwrap();
        assert_eq!(p0.scanned, 2);
        assert_eq!(
            p0.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["alpha", "Bravo"],
        );

        // Page 2 continues without overlap or gaps.
        let p1 = store.list_tracks_page(2, 2).unwrap();
        assert_eq!(
            p1.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Charlie", "Delta"],
        );

        // Last page is partial (one row left) → `scanned` < limit signals the
        // end; then empty past the end.
        let p2 = store.list_tracks_page(4, 2).unwrap();
        assert_eq!(p2.scanned, 1);
        assert_eq!(
            p2.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Echo"],
        );
        assert_eq!(store.list_tracks_page(6, 2).unwrap().scanned, 0);
        assert!(store.list_tracks_page(6, 2).unwrap().tracks.is_empty());

        // Paging the whole library equals one full ordered list.
        let full = store.list_tracks().unwrap();
        let paged: Vec<_> = [p0.tracks, p1.tracks, p2.tracks].concat();
        assert_eq!(
            full.iter().map(|t| &t.path).collect::<Vec<_>>(),
            paged.iter().map(|t| &t.path).collect::<Vec<_>>(),
        );

        // Negative args are clamped, not an error.
        assert_eq!(store.list_tracks_page(-5, -5).unwrap().tracks.len(), 0);
        assert!(!store.list_tracks_page(-5, 100).unwrap().tracks.is_empty());
    }

    #[test]
    fn page_hides_unreachable_files() {
        let store = MediaStore::open_in_memory().unwrap();

        // A real directory with a real file (reachable), like a connected drive.
        let dir = std::env::temp_dir().join("hm_avail_test_dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("present.mp3");
        std::fs::write(&real, b"x").unwrap();

        // "aaa" sorts first so the unreachable track lands inside page 1 — the
        // filter must drop it *and* still report it via `scanned`.
        store
            .upsert_track(&track("/no/such/drive/Music/gone.mp3", "aaa gone"))
            .unwrap();
        store
            .upsert_track(&track(real.to_str().unwrap(), "bbb present"))
            .unwrap();

        // Full page: both rows scanned, only the reachable one returned.
        let page = store.list_tracks_page(0, 10).unwrap();
        assert_eq!(page.scanned, 2, "both DB rows are scanned");
        assert_eq!(
            page.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["bbb present"],
            "the unreachable track is hidden",
        );

        // Even a page that contains *only* unreachable rows reports scanned>0 so
        // the caller keeps paging instead of stopping early.
        let first = store.list_tracks_page(0, 1).unwrap();
        assert_eq!(first.scanned, 1);
        assert!(first.tracks.is_empty(), "page 1 held only the gone track");

        // count_available reflects reachability; raw count still has both.
        assert_eq!(store.count_tracks().unwrap(), 2);
        assert_eq!(store.count_available_tracks().unwrap(), 1);

        let _ = std::fs::remove_dir_all(&dir);
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
