//! YouTube Music — playlists, library, and stream resolution.
//!
//! Two halves with deliberately different shapes:
//!
//! * **Metadata** (`ytmapi-rs`, async): playlists and their tracks, straight
//!   from YT Music's internal API. Needs only cookies — no yt-dlp, no PO token.
//!   So a user with no yt-dlp installed can still sign in and browse everything;
//!   only playback and downloads are gated.
//! * **Audio** (`yt-dlp`, sync): resolves a video id to a CDN URL + headers.
//!   Sync because it spawns a process, and because the engine's `StreamResolver`
//!   is a sync closure called from the decode worker.
//!
//! [`YtMusicState::stream_target`] deliberately mirrors `CloudState::stream_target`'s
//! `(url, headers)` signature. That's what lets YT Music reuse the whole existing
//! streaming stack — Range seeking, resume-on-drop, the gapless queue — instead
//! of growing a second one. Like Dropbox's temporary links, these URLs are
//! short-lived (they carry `expire=` and are pinned to the resolving IP), so
//! callers must re-resolve per attempt rather than cache them. The engine's
//! resolver already does exactly that.

pub mod cookies;
pub mod explore;
mod nav;
pub mod playlist;
pub mod ytdlp;

use cookies::{CookieFile, YtCookie};
use explore::{ExploreItem, ExploreKind, ExploreShelf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use ytdlp::{ProcessRunner, YtDlpError};
use ytmapi_rs::auth::BrowserToken;
use ytmapi_rs::common::{AlbumID, MoodCategoryParams, PlaylistID, YoutubeID};
use ytmapi_rs::parse::{ParseFrom, ProcessedResult};
use ytmapi_rs::query::{
    GetLibraryPlaylistsQuery, GetMoodPlaylistsQuery, GetPlaylistTracksQuery, PostMethod, PostQuery,
    Query,
};
use ytmapi_rs::YtMusic;

/// The library grid inside a `GetLibraryPlaylistsQuery` response.
const LIBRARY_GRID_ITEMS: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer\
    /content/sectionListRenderer/contents/0/gridRenderer/items";

/// How many playlists to fetch tracks for at once. Enough to hide latency on a
/// large library without hammering the API into rate-limiting us.
const PLAYLIST_FETCH_CONCURRENCY: usize = 6;

/// Continuation pages to follow before giving up on one playlist. At ~100 rows a
/// page this is far past any real playlist; it exists only so a token that
/// points at itself can't loop forever.
const PLAYLIST_PAGE_LIMIT: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtPlaylist {
    pub id: String,
    pub title: String,
    pub author: String,
    /// `None` when YT Music reports it in a form we don't recognise; the UI
    /// falls back to counting loaded tracks.
    pub track_count: Option<u32>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtTrack {
    pub video_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f64>,
    pub thumbnail: Option<String>,
    /// The playlist this track was listed under — drives the library's Folders
    /// facet, so playlists get grouping for free.
    pub playlist_id: String,
    pub playlist_title: String,
    /// YT Music marks region-blocked / removed tracks. They stay listed (so the
    /// playlist matches what the user sees on youtube) but can't be played.
    pub is_available: bool,
}

/// A row of Explore categories ("Moods & moments", "Genres", …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreSection {
    pub title: String,
    pub categories: Vec<ExploreCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreCategory {
    pub title: String,
    /// Opaque token identifying the category page; round-trips to the front end
    /// and back into `MoodCategoryParams`.
    pub params: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpInfo {
    pub present: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    /// Whether ffmpeg is around; without it downloads skip embedded tags/art.
    pub have_ffmpeg: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtMusicStatus {
    pub signed_in: bool,
    pub ytdlp: YtDlpInfo,
}

pub struct YtMusicState {
    /// Cached API handle. Rebuilt whenever the cookies change; `None` when
    /// signed out or not yet built.
    client: tokio::sync::Mutex<Option<Arc<YtMusic<BrowserToken>>>>,
    /// Cookies mirrored in memory so `stream_target` (sync, on the decode
    /// thread) never has to touch the keychain — that can block on a user
    /// prompt, which would stall audio.
    cookies: RwLock<Option<Vec<YtCookie>>>,
}

impl Default for YtMusicState {
    fn default() -> Self {
        Self::new()
    }
}

impl YtMusicState {
    pub fn new() -> Self {
        Self {
            client: tokio::sync::Mutex::new(None),
            cookies: RwLock::new(None),
        }
    }

    /// Builds the state, restoring any stored session.
    ///
    /// A keychain read failure is downgraded to "signed out" rather than
    /// propagated: a locked or unavailable keyring shouldn't stop the app from
    /// starting, and the user can always sign in again.
    pub fn load() -> Self {
        let state = Self::new();
        if let Ok(Some(stored)) = cookies::load() {
            let live = cookies::prune_expired(stored, cookies::now_secs());
            if cookies::is_signed_in(&live) {
                *state.cookies.write().unwrap() = Some(live);
            } else {
                // Session lapsed — clear it so the UI shows a clean signed-out
                // state instead of failing on first use.
                let _ = cookies::clear();
            }
        }
        state
    }

    pub fn signed_in(&self) -> bool {
        self.cookies.read().unwrap().is_some()
    }

    pub fn status(&self) -> YtMusicStatus {
        let runner = ProcessRunner::detect();
        YtMusicStatus {
            signed_in: self.signed_in(),
            ytdlp: YtDlpInfo {
                present: runner.is_some(),
                version: runner.as_ref().and_then(|r| r.version()),
                path: runner
                    .as_ref()
                    .map(|r| r.bin().to_string_lossy().into_owned()),
                have_ffmpeg: ytdlp::have_ffmpeg(),
            },
        }
    }

    /// Stores a freshly captured session.
    pub async fn sign_in(&self, captured: Vec<YtCookie>) -> Result<(), String> {
        let live = cookies::prune_expired(captured, cookies::now_secs());
        if !cookies::is_signed_in(&live) {
            return Err("Sign-in didn't complete — no YouTube session cookies were found.".into());
        }
        cookies::save(&live)?;
        *self.cookies.write().unwrap() = Some(live);
        // Force a rebuild so the next call uses the new session.
        *self.client.lock().await = None;
        // Prove the cookies actually work now, rather than failing later behind
        // an empty playlist list.
        self.client().await.map(|_| ())
    }

    pub async fn sign_out(&self) -> Result<(), String> {
        cookies::clear()?;
        *self.cookies.write().unwrap() = None;
        *self.client.lock().await = None;
        Ok(())
    }

    fn cookies_snapshot(&self) -> Option<Vec<YtCookie>> {
        self.cookies.read().unwrap().clone()
    }

    /// The API handle, built on first use and cached.
    async fn client(&self) -> Result<Arc<YtMusic<BrowserToken>>, String> {
        let mut slot = self.client.lock().await;
        if let Some(existing) = slot.as_ref() {
            return Ok(existing.clone());
        }
        let cookies = self
            .cookies_snapshot()
            .ok_or("Not signed in to YouTube Music.")?;
        let yt = YtMusic::from_cookie(cookies::header(&cookies))
            .await
            .map_err(|e| format!("YouTube Music rejected the session: {e}"))?;
        let yt = Arc::new(yt);
        *slot = Some(yt.clone());
        Ok(yt)
    }

    /// The user's playlists.
    ///
    /// Goes through `json_query` + [`pad_short_subtitles`] rather than
    /// `get_library_playlists`, because that helper's parser reads the track
    /// count from `/subtitle/runs/2/text` as a *mandatory* field and collects
    /// with `Result<_>` — so a single playlist whose subtitle is shorter than
    /// three runs (auto-mixes render just `["YouTube Music"]`) fails the whole
    /// listing, hiding every playlist the user has. Repairing the JSON keeps
    /// upstream doing the actual parsing; only that one brittle read is fixed.
    /// Equivalent in every other respect: `get_library_playlists` is itself just
    /// `query(GetLibraryPlaylistsQuery)`, one page with no continuation.
    pub async fn playlists(&self) -> Result<Vec<YtPlaylist>, String> {
        let yt = self.client().await?;
        let raw = yt
            .json_query(GetLibraryPlaylistsQuery)
            .await
            .map_err(|e| format!("Could not load playlists: {e}"))?;
        let mut json: Value = ytmapi_rs::json::from_json(raw)
            .map_err(|e| format!("Could not read the playlist listing: {e}"))?;
        pad_short_subtitles(&mut json);
        let body = serde_json::to_string(&json)
            .map_err(|e| format!("Could not re-encode the playlist listing: {e}"))?;
        let parsed = ytmapi_rs::process_json::<GetLibraryPlaylistsQuery, BrowserToken>(
            body,
            GetLibraryPlaylistsQuery,
        )
        .map_err(|e| format!("Could not load playlists: {e}"))?;
        Ok(parsed.into_iter().map(map_playlist).collect())
    }

    /// One playlist's tracks.
    pub async fn playlist_tracks(&self, playlist: &YtPlaylist) -> Result<Vec<YtTrack>, String> {
        let yt = self.client().await?;
        fetch_tracks(&yt, playlist).await
    }

    /* ---- explore ---- */

    /// The mood/genre categories YouTube offers ("Chill", "African", …).
    ///
    /// The one Explore call upstream gets right, so it stays upstream's.
    pub async fn explore_categories(&self) -> Result<Vec<ExploreSection>, String> {
        let yt = self.client().await?;
        let sections = yt
            .get_mood_categories()
            .await
            .map_err(|e| format!("Could not load Explore: {e}"))?;
        Ok(sections
            .into_iter()
            .map(|s| ExploreSection {
                title: s.section_name,
                categories: s
                    .mood_categories
                    .into_iter()
                    .map(|c| ExploreCategory {
                        title: c.title,
                        params: c.params.get_raw().to_string(),
                    })
                    .collect(),
            })
            .collect())
    }

    /// One category's shelves. See [`explore`] for why this parses the page
    /// itself instead of calling `get_mood_playlists`.
    pub async fn explore_page(&self, params: &str) -> Result<Vec<ExploreShelf>, String> {
        let yt = self.client().await?;
        let raw = yt
            .json_query(GetMoodPlaylistsQuery::new(MoodCategoryParams::from_raw(
                params,
            )))
            .await
            .map_err(|e| format!("Could not load that category: {e}"))?;
        let json: Value = ytmapi_rs::json::from_json(raw)
            .map_err(|e| format!("Could not read that category: {e}"))?;
        Ok(explore::parse_mood_page(&json))
    }

    /// The tracks behind one Explore item, ready to queue.
    ///
    /// Nothing is cached: Explore is YouTube's live catalog and its whole value
    /// is being current, so every open is a fresh read.
    pub async fn explore_tracks(&self, item: &ExploreItem) -> Result<Vec<YtTrack>, String> {
        match item.kind {
            ExploreKind::Playlist => {
                // `id` is already the VL-prefixed browse id the query wants.
                let playlist = YtPlaylist {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    author: String::new(),
                    track_count: None,
                    thumbnail: item.thumbnail.clone(),
                };
                let yt = self.client().await?;
                fetch_tracks(&yt, &playlist).await
            }
            ExploreKind::Album => self.album_tracks(&item.id, &item.title).await,
        }
    }

    /// One album's tracks.
    ///
    /// `get_album` returns no per-track artwork or artist, so the album's own
    /// cover and artist stand in for every track — which is what an album *is*,
    /// and what the queue and Albums facet want anyway.
    async fn album_tracks(&self, album_id: &str, fallback_title: &str) -> Result<Vec<YtTrack>, String> {
        let yt = self.client().await?;
        let album = yt
            .get_album(AlbumID::from_raw(album_id))
            .await
            .map_err(|e| format!("Could not load \"{fallback_title}\": {e}"))?;
        let artist = join_artists(&album.artists);
        let cover = best_thumbnail(&album.thumbnails);
        Ok(album
            .tracks
            .into_iter()
            .map(|t| YtTrack {
                video_id: t.video_id.get_raw().to_string(),
                title: t.title,
                artist: artist.clone(),
                album: (!album.title.is_empty()).then(|| album.title.clone()),
                duration_secs: parse_duration(&t.duration),
                thumbnail: cover.clone(),
                playlist_id: album_id.to_string(),
                playlist_title: album.title.clone(),
                // `get_album` doesn't report per-track availability; a track that
                // turns out to be blocked fails at resolve time like any other.
                is_available: true,
            })
            .collect())
    }

    /* ---- library ---- */

    /// Every track across every playlist — what the library view lists.
    ///
    /// Playlists are fetched concurrently (bounded by
    /// [`PLAYLIST_FETCH_CONCURRENCY`]); a playlist that fails is skipped rather
    /// than failing the whole load, so one broken playlist can't hide a library.
    pub async fn all_tracks(&self) -> Result<(Vec<YtPlaylist>, Vec<YtTrack>), String> {
        let yt = self.client().await?;
        let playlists = self.playlists().await?;

        let sem = Arc::new(tokio::sync::Semaphore::new(PLAYLIST_FETCH_CONCURRENCY));
        let mut set = tokio::task::JoinSet::new();
        for pl in playlists.clone() {
            let yt = yt.clone();
            let sem = sem.clone();
            set.spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                fetch_tracks(&yt, &pl).await.ok()
            });
        }

        let mut tracks = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(Some(mut got)) = res {
                tracks.append(&mut got);
            }
        }
        Ok((playlists, tracks))
    }

    /// Resolves a track to a directly-playable `(url, headers)`.
    ///
    /// Sync on purpose: the engine's `StreamResolver` is a sync closure invoked
    /// from the decode worker, and this only spawns a process.
    pub fn stream_target(&self, video_id: &str) -> Result<(String, Vec<(String, String)>), String> {
        let target = self.resolve(video_id)?;
        Ok((target.url, target.headers))
    }

    /// Like [`Self::stream_target`], but keeps the format details the caller may
    /// want (the container ext is a demuxer hint).
    pub fn resolve(&self, video_id: &str) -> Result<ytdlp::StreamTarget, String> {
        let runner = ProcessRunner::detect().ok_or_else(|| YtDlpError::NotInstalled.to_string())?;
        let cookies = self.cookies_snapshot();
        let file = cookie_file(cookies.as_deref())?;
        ytdlp::resolve(&runner, video_id, file.as_ref().map(CookieFile::path))
            .map_err(|e| e.to_string())
    }

    /// Downloads a track into `dest_dir`, returning the written path.
    pub fn download(
        &self,
        video_id: &str,
        dest_dir: &Path,
        on_progress: impl FnMut(ytdlp::Progress),
    ) -> Result<PathBuf, String> {
        let runner = ProcessRunner::detect().ok_or_else(|| YtDlpError::NotInstalled.to_string())?;
        std::fs::create_dir_all(dest_dir)
            .map_err(|e| format!("Could not create the downloads folder: {e}"))?;
        let cookies = self.cookies_snapshot();
        let file = cookie_file(cookies.as_deref())?;
        ytdlp::download(
            &runner,
            video_id,
            dest_dir,
            file.as_ref().map(CookieFile::path),
            ytdlp::have_ffmpeg(),
            on_progress,
        )
        .map_err(|e| e.to_string())
    }
}

/// Materialises cookies for yt-dlp, if we have any. Signed-out use still works
/// for public tracks, just without Premium's PO-token exemption.
fn cookie_file(cookies: Option<&[YtCookie]>) -> Result<Option<CookieFile>, String> {
    match cookies {
        Some(c) if !c.is_empty() => CookieFile::new(c).map(Some),
        _ => Ok(None),
    }
}

/// One continuation of a playlist listing: the same browse POST as the first
/// page, plus the token.
///
/// Exists because `GetContinuationsQuery` can only be built via
/// `from_first_result`, which runs the row parser that fails on a channel-less
/// row — so upstream stops paging exactly the playlists that most need it. This
/// asks for the next page with a token we found ourselves. The wire format is
/// upstream's own (`ctoken` + `continuation`, same header and path).
struct PlaylistContinuation<'a> {
    base: &'a GetPlaylistTracksQuery<'a>,
    token: String,
}

impl PostQuery for PlaylistContinuation<'_> {
    fn header(&self) -> serde_json::Map<String, Value> {
        self.base.header()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![
            ("ctoken", self.token.as_str().into()),
            ("continuation", self.token.as_str().into()),
        ]
    }
    fn path(&self) -> &str {
        self.base.path()
    }
}

/// `json_query` hands back the response JSON before parsing, so the typed output
/// is never constructed — this only exists to satisfy `Query`.
#[derive(Debug)]
struct RawPage;

impl ParseFrom<PlaylistContinuation<'_>> for RawPage {
    fn parse_from(_: ProcessedResult<PlaylistContinuation<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(RawPage)
    }
}

impl<A: ytmapi_rs::auth::AuthToken> Query<A> for PlaylistContinuation<'_> {
    type Output = RawPage;
    type Method = PostMethod;
}

/// Every track in a playlist, following continuations to the end.
///
/// Two upstream limits made this the wrong shape twice over, so it takes the raw
/// pages and reads the rows itself (see [`playlist`]):
///
/// * `get_playlist_tracks` returns only the **first page** — ~100 tracks — with
///   no hint that more exist, so a 389-track playlist silently loaded 100.
///   `raw_json_stream` follows the continuation tokens.
/// * Its row parser demands a channel id that collaboration credits don't have,
///   and fails the whole page over one such row.
///
/// A page that can't even be read as JSON ends the walk rather than discarding
/// what came before: a partial playlist beats none, and dropping it silently is
/// the truncation bug in a new costume.
async fn fetch_tracks(
    yt: &YtMusic<BrowserToken>,
    playlist: &YtPlaylist,
) -> Result<Vec<YtTrack>, String> {
    let query = GetPlaylistTracksQuery::new(PlaylistID::from_raw(playlist.id.clone()));

    let first = yt
        .json_query::<GetPlaylistTracksQuery>(&query)
        .await
        .map_err(|e| format!("Could not load \"{}\": {e}", playlist.title))?;
    let json: Value = ytmapi_rs::json::from_json(first)
        .map_err(|e| format!("Could not read \"{}\": {e}", playlist.title))?;

    let mut tracks = playlist::parse_page(&json, playlist);
    let mut token = playlist::next_page_token(&json);

    // Bounded so a token that keeps pointing at itself can't spin forever; well
    // clear of any real playlist at ~100 rows a page.
    for _ in 0..PLAYLIST_PAGE_LIMIT {
        let Some(next) = token.take() else { break };
        let cont = PlaylistContinuation {
            base: &query,
            token: next,
        };
        // A failed continuation keeps the pages already read: a partial playlist
        // beats none, and dropping it silently is the truncation bug in a new
        // costume.
        let Ok(raw) = yt.json_query::<PlaylistContinuation>(&cont).await else {
            break;
        };
        let Ok(page) = ytmapi_rs::json::from_json::<Value>(raw) else {
            break;
        };
        let rows = playlist::parse_page(&page, playlist);
        // No rows and no token → nothing more is coming.
        if rows.is_empty() && playlist::next_page_token(&page).is_none() {
            break;
        }
        tracks.extend(rows);
        token = playlist::next_page_token(&page);
    }
    Ok(tracks)
}

/// Pads every library-grid subtitle out to the three runs upstream's parser
/// insists on, returning how many needed it.
///
/// A normal entry reads `["Bruno", " • ", "134 tracks"]`, but YT Music renders
/// auto-generated ones (`"Archive Mix"`) as a bare `["YouTube Music"]` — no
/// count. Upstream takes `/runs/0/text` as the author and `/runs/2/text` as the
/// count and hard-errors when the latter is missing, so one such playlist blanks
/// the entire library. Padding with empty runs preserves the author and leaves
/// the count as `""`, which [`parse_track_count`] reports as `None` — the
/// "unknown, count the loaded tracks" case [`YtPlaylist::track_count`] already
/// documents.
///
/// Only widens: a subtitle that already has three or more runs is untouched, and
/// so is one with no runs at all (no author to salvage — upstream can reject it
/// as it always has).
fn pad_short_subtitles(json: &mut Value) -> usize {
    let Some(items) = json
        .pointer_mut(LIBRARY_GRID_ITEMS)
        .and_then(Value::as_array_mut)
    else {
        return 0;
    };
    let mut padded = 0;
    for item in items.iter_mut() {
        let Some(runs) = item
            .pointer_mut("/musicTwoRowItemRenderer/subtitle/runs")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        if runs.is_empty() || runs.len() >= 3 {
            continue;
        }
        while runs.len() < 3 {
            runs.push(json!({ "text": "" }));
        }
        padded += 1;
    }
    padded
}

fn map_playlist(p: ytmapi_rs::parse::LibraryPlaylist) -> YtPlaylist {
    YtPlaylist {
        id: p.playlist_id.get_raw().to_string(),
        title: p.title,
        author: p.author,
        track_count: parse_track_count(&p.tracks),
        thumbnail: best_thumbnail(&p.thumbnails),
    }
}

fn join_artists(artists: &[ytmapi_rs::parse::ParsedSongArtist]) -> Option<String> {
    let names: Vec<&str> = artists
        .iter()
        .map(|a| a.name.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// Picks the largest thumbnail. They're ordered small-to-large in practice, but
/// that isn't documented, so choose explicitly.
fn best_thumbnail(thumbs: &[ytmapi_rs::common::Thumbnail]) -> Option<String> {
    thumbs
        .iter()
        .max_by_key(|t| t.width * t.height)
        .map(|t| t.url.clone())
}

/// Parses YT Music's `M:SS` / `H:MM:SS` duration.
///
/// Returns `None` rather than a wrong number for anything unexpected — a null
/// duration renders as blank, while a bogus one would corrupt seek bars.
pub(crate) fn parse_duration(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut secs: u64 = 0;
    let mut parts = 0;
    for part in raw.split(':') {
        let v: u64 = part.trim().parse().ok()?;
        secs = secs.checked_mul(60)?.checked_add(v)?;
        parts += 1;
    }
    (1..=3).contains(&parts).then_some(secs as f64)
}

/// Pulls a count out of YT Music's `"12 songs"` / `"1 song"` label.
fn parse_track_count(raw: &str) -> Option<u32> {
    raw.split_whitespace().next()?.replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mm_ss() {
        assert_eq!(parse_duration("3:33"), Some(213.0));
        assert_eq!(parse_duration("0:07"), Some(7.0));
    }

    #[test]
    fn parses_hh_mm_ss() {
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
    }

    #[test]
    fn parses_bare_seconds() {
        assert_eq!(parse_duration("45"), Some(45.0));
    }

    #[test]
    fn rejects_junk_duration() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("live"), None);
        assert_eq!(parse_duration("3:xx"), None);
        assert_eq!(parse_duration("1:2:3:4"), None);
    }

    #[test]
    fn parses_track_counts() {
        assert_eq!(parse_track_count("12 songs"), Some(12));
        assert_eq!(parse_track_count("1 song"), Some(1));
        assert_eq!(parse_track_count("1,234 songs"), Some(1234));
        assert_eq!(parse_track_count(""), None);
        assert_eq!(parse_track_count("lots of songs"), None);
    }

    #[test]
    fn joins_artists_skipping_blanks() {
        let artists = vec![
            ytmapi_rs::parse::ParsedSongArtist {
                name: "A".into(),
                id: None,
            },
            ytmapi_rs::parse::ParsedSongArtist {
                name: String::new(),
                id: None,
            },
            ytmapi_rs::parse::ParsedSongArtist {
                name: "B".into(),
                id: None,
            },
        ];
        assert_eq!(join_artists(&artists), Some("A, B".to_string()));
        assert_eq!(join_artists(&[]), None);
    }

    #[test]
    fn picks_largest_thumbnail() {
        let thumbs = vec![
            ytmapi_rs::common::Thumbnail {
                height: 60,
                width: 60,
                url: "small".into(),
            },
            ytmapi_rs::common::Thumbnail {
                height: 544,
                width: 544,
                url: "large".into(),
            },
        ];
        assert_eq!(best_thumbnail(&thumbs), Some("large".to_string()));
        assert_eq!(best_thumbnail(&[]), None);
    }

    /// A library response whose grid holds one item per entry: `None` is an item
    /// with no subtitle node at all, `Some(runs)` one with those run texts.
    fn grid(subtitles: &[Option<Vec<&str>>]) -> Value {
        let items: Vec<Value> = subtitles
            .iter()
            .map(|runs| match runs {
                None => json!({ "musicTwoRowItemRenderer": {} }),
                Some(texts) => json!({
                    "musicTwoRowItemRenderer": {
                        "subtitle": {
                            "runs": texts
                                .iter()
                                .map(|t| json!({ "text": t }))
                                .collect::<Vec<_>>(),
                        }
                    }
                }),
            })
            .collect();
        json!({
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": {
                "content": { "sectionListRenderer": { "contents": [{ "gridRenderer": {
                    "items": items
                }}]}}
            }}]}}
        })
    }

    fn runs_at(json: &Value, i: usize) -> Vec<String> {
        json.pointer(LIBRARY_GRID_ITEMS).unwrap().as_array().unwrap()[i]
            .pointer("/musicTwoRowItemRenderer/subtitle/runs")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["text"].as_str().unwrap().to_string())
            .collect()
    }

    /// The exact shape that used to blank the whole library: YT Music renders an
    /// auto-mix's subtitle as a lone `["YouTube Music"]`.
    #[test]
    fn pads_auto_mix_subtitle_so_the_count_read_resolves() {
        let mut json = grid(&[Some(vec!["YouTube Music"])]);
        assert_eq!(pad_short_subtitles(&mut json), 1);
        // Author (run 0) survives; the count upstream demands is now present and
        // empty, which reads back as "unknown" rather than failing the listing.
        assert_eq!(runs_at(&json, 0), ["YouTube Music", "", ""]);
        assert_eq!(parse_track_count(""), None);
    }

    #[test]
    fn leaves_well_formed_subtitles_untouched() {
        let mut json = grid(&[
            Some(vec!["Bruno", " • ", "134 tracks"]),
            Some(vec!["Made for ", "Bruno", " • ", "100 songs"]),
        ]);
        assert_eq!(pad_short_subtitles(&mut json), 0);
        assert_eq!(runs_at(&json, 0), ["Bruno", " • ", "134 tracks"]);
        assert_eq!(runs_at(&json, 1), ["Made for ", "Bruno", " • ", "100 songs"]);
    }

    /// Nothing to salvage in either case, so they stay as they are and upstream
    /// keeps whatever verdict it had on them.
    #[test]
    fn leaves_empty_and_absent_subtitles_alone() {
        let mut json = grid(&[Some(vec![]), None]);
        assert_eq!(pad_short_subtitles(&mut json), 0);
        assert!(runs_at(&json, 0).is_empty());
    }

    #[test]
    fn counts_only_the_items_it_padded() {
        let mut json = grid(&[
            Some(vec!["Auto playlist"]),
            Some(vec!["Bruno", " • ", "1 track"]),
            Some(vec!["YouTube Music"]),
        ]);
        assert_eq!(pad_short_subtitles(&mut json), 2);
    }

    /// A response that isn't the library grid must not panic or be mangled.
    #[test]
    fn ignores_a_response_without_a_grid() {
        let mut json = json!({ "contents": {} });
        assert_eq!(pad_short_subtitles(&mut json), 0);
        assert_eq!(json, json!({ "contents": {} }));
    }

    /// The real listing must parse end to end. The fixtures above only prove the
    /// repair we know about; YT Music can change a subtitle's shape at any time
    /// and the failure is total (one odd playlist hides the whole library), so
    /// this is the only check that catches the next one. Needs a signed-in
    /// session in the OS keychain — sign in through the app first.
    ///
    /// Every Explore category must yield shelves, and albums must actually be
    /// reachable. This is the test that earns the hand-written parser: upstream
    /// managed 19 of 44 categories and zero albums, and nothing but a live sweep
    /// would have told us. Slow (one request per category) and network-bound, so
    /// it's `--ignored` like the rest.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_explore_sweeps_every_category() {
        let state = YtMusicState::load();
        if !state.signed_in() {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        }
        let sections = state.explore_categories().await.expect("categories must load");
        let total: usize = sections.iter().map(|s| s.categories.len()).sum();
        assert!(total > 0, "Explore should offer categories");

        let mut empty = Vec::new();
        let mut albums = 0;
        let mut playlists = 0;
        for section in &sections {
            for cat in &section.categories {
                let shelves = state
                    .explore_page(&cat.params)
                    .await
                    .expect("a category page must never error");
                if shelves.is_empty() {
                    empty.push(cat.title.clone());
                    continue;
                }
                for shelf in &shelves {
                    for item in &shelf.items {
                        match item.kind {
                            ExploreKind::Album => albums += 1,
                            ExploreKind::Playlist => playlists += 1,
                        }
                    }
                }
            }
        }
        eprintln!(
            "{total} categories: {playlists} playlists, {albums} albums; {} empty {empty:?}",
            empty.len()
        );
        assert!(
            empty.is_empty(),
            "every category should yield at least one shelf, these didn't: {empty:?}"
        );
        assert!(albums > 0, "Explore should surface albums — that's the point");
    }

    /// Opening an item must produce queueable tracks — for both kinds. Uses the
    /// first album and first playlist Explore actually offers today.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_explore_items_open_into_tracks() {
        let state = YtMusicState::load();
        if !state.signed_in() {
            eprintln!("skipping: no session in the keychain");
            return;
        }
        let sections = state.explore_categories().await.expect("categories");
        let mut album: Option<ExploreItem> = None;
        let mut playlist: Option<ExploreItem> = None;
        'outer: for section in &sections {
            for cat in &section.categories {
                let Ok(shelves) = state.explore_page(&cat.params).await else {
                    continue;
                };
                for shelf in shelves {
                    for item in shelf.items {
                        match item.kind {
                            ExploreKind::Album if album.is_none() => album = Some(item),
                            ExploreKind::Playlist if playlist.is_none() => playlist = Some(item),
                            _ => {}
                        }
                    }
                }
                if album.is_some() && playlist.is_some() {
                    break 'outer;
                }
            }
        }

        let album = album.expect("Explore should offer an album");
        let tracks = state.explore_tracks(&album).await.expect("album must open");
        eprintln!("album {:?} -> {} tracks", album.title, tracks.len());
        assert!(!tracks.is_empty(), "an album should have tracks");
        assert!(
            tracks.iter().all(|t| !t.video_id.is_empty()),
            "every track needs a video id to be playable"
        );

        let playlist = playlist.expect("Explore should offer a playlist");
        let tracks = state
            .explore_tracks(&playlist)
            .await
            .expect("playlist must open");
        eprintln!("playlist {:?} -> {} tracks", playlist.title, tracks.len());
        assert!(!tracks.is_empty(), "a playlist should have tracks");
    }

    /// Every playlist must load *in full*, and none may fail.
    ///
    /// Guards two bugs that between them hid 60% of a real library while every
    /// unit test stayed green: `get_playlist_tracks` returns only the first ~100
    /// rows, and its row parser dies on a credit with no channel link — which
    /// also stops upstream's pager, so the playlists needing continuations most
    /// were the ones that never got them. Both are invisible without a real
    /// account: fixtures can't tell you YouTube stopped at page one.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_playlists_load_completely() {
        let state = YtMusicState::load();
        if !state.signed_in() {
            eprintln!("skipping: no session in the keychain");
            return;
        }
        let playlists = state.playlists().await.expect("playlists must list");
        assert!(!playlists.is_empty(), "a signed-in library should have playlists");

        let mut total = 0usize;
        let mut short = Vec::new();
        for pl in &playlists {
            let tracks = state
                .playlist_tracks(pl)
                .await
                .unwrap_or_else(|e| panic!("\"{}\" must load: {e}", pl.title));
            total += tracks.len();
            // Podcast episodes and uploads are dropped on purpose, so a playlist
            // may legitimately come in a little under its advertised count; only
            // a real shortfall (a missed page is ~100) is a failure.
            if let Some(claimed) = pl.track_count {
                if tracks.len() + 10 < claimed as usize {
                    short.push(format!("{} got {} of {claimed}", pl.title, tracks.len()));
                }
            }
        }
        eprintln!("{} playlists, {total} tracks", playlists.len());
        assert!(
            short.is_empty(),
            "playlists loaded short — pagination or row parsing regressed: {short:?}"
        );
    }

    /// Run with `cargo test -p hm-ytmusic -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_library_listing_parses() {
        let state = YtMusicState::load();
        if !state.signed_in() {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        }
        let playlists = state.playlists().await.expect("the listing must parse");
        eprintln!("parsed {} playlists", playlists.len());
        assert!(!playlists.is_empty(), "a signed-in library should list something");
    }
}
