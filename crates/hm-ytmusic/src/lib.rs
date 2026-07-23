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
//! of growing a second one.
//!
//! Unlike Dropbox's temporary links, these are **not** short-lived: googlevideo
//! stamps `expire=` about six hours out, so the url outlives the track by a wide
//! margin and is cached until then. Resolving is a yt-dlp process start — an
//! interpreter and every extractor loaded before the network is touched, ~5s —
//! and doing it per play, or per retry, put all of that in the gap between two
//! tracks.
//!
//! What they *are* is pinned to the resolving IP, and nothing in the url says so.
//! A link can therefore die well before its stated deadline, which is why the
//! cache can only be trusted until someone reports otherwise: the resolver
//! contract carries `fresh` for exactly that, and only a retry sets it.

pub mod cookies;
pub mod explore;
mod innertube;
mod nav;
pub mod playlist;
mod radio;
pub mod search;
pub mod ytdlp;

use cookies::{CookieFile, YtCookie};
use explore::{ExploreItem, ExploreKind, ExploreShelf};
use search::SearchFilter;

pub use radio::RadioBatch;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use ytdlp::{ProcessRunner, YtDlpError};
use ytmapi_rs::auth::BrowserToken;
use ytmapi_rs::common::{
    AlbumID, ArtistChannelID, MoodCategoryParams, PlaylistID, VideoID, YoutubeID,
};
use ytmapi_rs::parse::{ParseFrom, ProcessedResult};
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, BasicSearch, PlaylistsFilter, SearchQuery, SongsFilter,
    VideosFilter,
};
use ytmapi_rs::query::{
    GetArtistQuery, GetLibraryPlaylistsQuery, GetMoodPlaylistsQuery, GetPlaylistTracksQuery,
    GetSearchSuggestionsQuery, GetWatchPlaylistQuery, PostMethod, PostQuery, Query,
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
    /// Whether there's real footage to watch.
    ///
    /// A song (`MUSIC_VIDEO_TYPE_ATV`) is an audio entity: YouTube still serves
    /// "video" renditions for it, but they're a **square still image** — 1080×1080
    /// at ~95 kbps, i.e. the cover art you already have, re-downloaded. Only
    /// music videos (OMV/UGC/…) have anything worth showing, so the UI offers the
    /// video toggle on those alone rather than promising a picture and
    /// delivering a photograph.
    pub has_video: bool,
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
    /// Whether the session is worth trying *first* when resolving a stream.
    ///
    /// Starts true and flips to whatever last worked. Where accounts exist that
    /// YouTube serves no playable format to, every track otherwise paid for a
    /// doomed yt-dlp spawn before the one that works — seconds, in front of the
    /// play button. Both attempts always remain available, so a private track
    /// still resolves; this only decides the order.
    session_first: std::sync::atomic::AtomicBool,
    /// Resolved stream urls, kept until the CDN's own `expire=` says not to.
    ///
    /// A resolve is a yt-dlp process start — a Python interpreter and extractor
    /// import before a single byte moves — and measures ~5s. That cost landed
    /// between two tracks, where it *is* the gap the listener hears. The urls are
    /// good for ~6 hours, so paying it more than once a track is pure waste.
    ///
    /// Keyed by video id. Bounded by pruning the already-expired on write: an
    /// entry is only useful while live, so what makes one stale is also what
    /// makes it collectable and no separate eviction policy is needed.
    resolved: RwLock<std::collections::HashMap<String, ytdlp::StreamTarget>>,
    /// Disk-restored urls, quarantined until probed.
    ///
    /// A restart often means a network change, and googlevideo urls are
    /// IP-bound — so nothing in here may be served without one cheap probe
    /// first (see `live_or_probed_target`). Same-session entries never pass
    /// through this map and keep their unprobed ~µs hits.
    restored: TargetCache,
    /// Bumped whenever what a snapshot would contain changes — `remember_target`,
    /// a probe dropping a restored entry, and `forget()` (a dead url must leave
    /// the on-disk snapshot too). The disk saver polls it to skip writes when
    /// nothing moved. Restore doesn't bump: that state came FROM the file.
    cache_generation: std::sync::atomic::AtomicU64,
    /// Resolved *video* urls, on the same terms as [`Self::resolved`].
    ///
    /// A separate map because the same video id resolves to two different urls —
    /// the m4a the engine decodes, and the video-only rendition the picture
    /// shows. One map keyed by id would have them evict and answer for each
    /// other.
    ///
    /// This one is arguably the more important of the two: the audio url is
    /// prefetched a track ahead, but the video url is resolved on demand, so its
    /// cold cost lands *while the user is watching the spinner*.
    resolved_video: RwLock<std::collections::HashMap<String, ytdlp::StreamTarget>>,
    /// Where yt-dlp lives, once found.
    ///
    /// `detect()` stats every entry on `PATH` and ran on every resolve. Only a
    /// hit is remembered: a miss has to stay cheap to retry, or installing
    /// yt-dlp wouldn't take effect until a restart.
    yt_dlp_bin: RwLock<Option<PathBuf>>,
}

/// How much of a resolved url's life to leave unused.
///
/// The url must outlive not just the click but the whole track played through it
/// — a reconnection near the end still re-opens with the original url. Tracks run
/// long (a mix can pass an hour), so this is generous: against a ~6 hour lifetime
/// it costs little, and a url dying mid-playback costs a lot.
const EXPIRY_MARGIN_SECS: u64 = 90 * 60;

/// Seconds since the epoch, or `None` if the clock is before it.
fn now_secs() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Whether `target` will still be honoured long enough to play a track through.
fn is_fresh(target: &ytdlp::StreamTarget, now: u64) -> bool {
    // No stated deadline means we don't know one; a guessed lifetime is how a
    // cache starts serving dead urls.
    target
        .expires_at
        .is_some_and(|exp| exp.saturating_sub(now) > EXPIRY_MARGIN_SECS)
}

/// A still-live entry from one of the resolve caches.
type TargetCache = RwLock<std::collections::HashMap<String, ytdlp::StreamTarget>>;

/// One blocking client for every probe: connection reuse, and no per-probe
/// construction cost. Separate from hm-audio's stream client by crate
/// boundary — the probe's TLS warm doesn't transfer there; accepted, the
/// probe's job is validity, not warming.
fn probe_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("default TLS config must build")
    })
}

/// Whether `target` still answers from THIS network. Two bytes, ranged — the
/// cheapest question the CDN accepts (~100–300ms).
fn probe_ok(target: &ytdlp::StreamTarget) -> bool {
    let mut req = probe_client().get(&target.url).header("Range", "bytes=0-1");
    for (k, v) in &target.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    match req.send() {
        Ok(r) => {
            let s = r.status();
            s == reqwest::StatusCode::OK || s == reqwest::StatusCode::PARTIAL_CONTENT
        }
        Err(_) => false,
    }
}

fn cached_in(cache: &TargetCache, video_id: &str) -> Option<ytdlp::StreamTarget> {
    let now = now_secs()?;
    let cache = cache.read().ok()?;
    cache.get(video_id).filter(|t| is_fresh(t, now)).cloned()
}

/// Files a resolved url under its video id, and drops whatever has expired.
fn remember_in(cache: &TargetCache, video_id: &str, target: &ytdlp::StreamTarget) {
    // Nothing to reason about without a deadline, and a url we can't date is
    // one we can't safely re-serve.
    if target.expires_at.is_none() {
        return;
    }
    let Some(now) = now_secs() else { return };
    let Ok(mut cache) = cache.write() else {
        return;
    };
    cache.retain(|_, t| is_fresh(t, now));
    cache.insert(video_id.to_string(), target.clone());
}

/// On-disk shape of the persisted url cache. Versioned so a future
/// `StreamTarget` change can't half-parse an old file into wrong urls.
#[derive(serde::Serialize, serde::Deserialize)]
struct UrlCacheFile {
    version: u32,
    entries: std::collections::HashMap<String, ytdlp::StreamTarget>,
}

const URL_CACHE_VERSION: u32 = 1;

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
            session_first: std::sync::atomic::AtomicBool::new(true),
            resolved: RwLock::new(std::collections::HashMap::new()),
            restored: RwLock::new(std::collections::HashMap::new()),
            cache_generation: std::sync::atomic::AtomicU64::new(0),
            resolved_video: RwLock::new(std::collections::HashMap::new()),
            yt_dlp_bin: RwLock::new(None),
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
        // A new session earns a fresh chance at going first.
        self.session_first
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Force a rebuild so the next call uses the new session.
        *self.client.lock().await = None;
        // Prove the cookies actually work now, rather than failing later behind
        // an empty playlist list.
        self.client().await.map(|_| ())
    }

    pub async fn sign_out(&self) -> Result<(), String> {
        cookies::clear()?;
        *self.cookies.write().unwrap() = None;
        self.session_first
            .store(true, std::sync::atomic::Ordering::Relaxed);
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
        Ok(explore::parse_page(&json))
    }

    /* ---- search ---- */

    /// Searching YouTube's catalog, one filter at a time.
    ///
    /// Every filter is its own request — that's how YT Music's own search works,
    /// and the unfiltered query answers with a different, shallower set (a top
    /// result and a handful of each kind) rather than with everything.
    ///
    /// See [`search`] for why the typed `search_songs`/`search_albums` helpers
    /// are unused: each collects with `Result<_>` over rows whose play count and
    /// byline it treats as mandatory, so one odd result returns none.
    pub async fn search(&self, query: &str, filter: &str) -> Result<Vec<ExploreShelf>, String> {
        let query = query.trim();
        // Nothing typed is not an error, and asking YouTube about "" would earn
        // a page of results for nothing.
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let yt = self.client().await?;
        // Each arm builds a differently-typed query, so the JSON is fetched
        // inside the match and read once outside it.
        let raw = match SearchFilter::parse(filter) {
            SearchFilter::Top => yt.json_query(SearchQuery::<BasicSearch>::from(query)).await,
            SearchFilter::Songs => {
                yt.json_query(SearchQuery::new_filtered(query, SongsFilter))
                    .await
            }
            SearchFilter::Videos => {
                yt.json_query(SearchQuery::new_filtered(query, VideosFilter))
                    .await
            }
            SearchFilter::Albums => {
                yt.json_query(SearchQuery::new_filtered(query, AlbumsFilter))
                    .await
            }
            SearchFilter::Artists => {
                yt.json_query(SearchQuery::new_filtered(query, ArtistsFilter))
                    .await
            }
            SearchFilter::Playlists => {
                yt.json_query(SearchQuery::new_filtered(query, PlaylistsFilter))
                    .await
            }
        }
        .map_err(|e| format!("Could not search for \"{query}\": {e}"))?;
        let json: Value = ytmapi_rs::json::from_json(raw)
            .map_err(|e| format!("Could not read the results for \"{query}\": {e}"))?;
        Ok(search::parse_search_page(&json))
    }

    /// What YouTube would complete a half-typed query with.
    ///
    /// A failure is an empty list, not an error: this fires on every keystroke,
    /// and a type-ahead that can raise an error dialog is worse than one that
    /// occasionally offers nothing.
    pub async fn search_suggestions(&self, query: &str) -> Vec<String> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(yt) = self.client().await else {
            return Vec::new();
        };
        let Ok(raw) = yt.json_query(GetSearchSuggestionsQuery::new(query)).await else {
            return Vec::new();
        };
        match ytmapi_rs::json::from_json::<Value>(raw) {
            Ok(json) => search::parse_suggestions(&json),
            Err(_) => Vec::new(),
        }
    }

    /// An artist's page — top songs, albums, singles, videos and the rest.
    ///
    /// Returns the shelves as YouTube ordered them, so the view renders what YT
    /// Music renders rather than a re-grouping of it.
    ///
    /// `get_artist` exists upstream and is avoided for the usual reason: it
    /// insists on a fixed set of sections and fails the page when one is absent,
    /// and an artist with no albums is an ordinary artist.
    pub async fn artist_page(&self, browse_id: &str) -> Result<Vec<ExploreShelf>, String> {
        let yt = self.client().await?;
        let raw = yt
            .json_query(GetArtistQuery::new(ArtistChannelID::from_raw(browse_id)))
            .await
            .map_err(|e| format!("Could not load that artist: {e}"))?;
        let json: Value = ytmapi_rs::json::from_json(raw)
            .map_err(|e| format!("Could not read that artist: {e}"))?;
        Ok(explore::parse_page(&json))
    }

    /// The endless "up next" YT Music derives from one song — its radio.
    ///
    /// This is the same `next` call the YT Music client makes: playlist
    /// `RDAMVM<videoId>`, raw id, **no `VL` prefix** (`next` is the opposite of
    /// `browse` on this). The typed parser upstream would hard-fail the page on
    /// one odd row and skip no unplayable ones, so the JSON is read by
    /// [`radio::parse_radio_page`] instead.
    pub async fn radio(&self, video_id: &str) -> Result<RadioBatch, String> {
        let yt = self.client().await?;
        let query = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(video_id));
        let raw = yt
            .json_query::<GetWatchPlaylistQuery<VideoID>>(&query)
            .await
            .map_err(|e| format!("Radio unavailable: {e}"))?;
        let json: Value =
            ytmapi_rs::json::from_json(raw).map_err(|e| format!("Radio unreadable: {e}"))?;
        Ok(radio::parse_radio_page(&json))
    }

    /// The next page of a radio: the same body as [`Self::radio`] plus the token
    /// the previous page returned. Needs the seed's `video_id` because the wire
    /// format re-POSTs the full body — the token alone is not a request.
    pub async fn radio_continue(&self, video_id: &str, token: &str) -> Result<RadioBatch, String> {
        let yt = self.client().await?;
        let base = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(video_id));
        let cont = WatchContinuation {
            base: &base,
            token: token.to_string(),
        };
        let raw = yt
            .json_query::<WatchContinuation>(&cont)
            .await
            .map_err(|e| format!("Radio continuation failed: {e}"))?;
        let json: Value =
            ytmapi_rs::json::from_json(raw).map_err(|e| format!("Radio unreadable: {e}"))?;
        Ok(radio::parse_radio_page(&json))
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
            // A row already said everything a track needs, so playing one costs
            // no request at all — the id it carries is the video id.
            ExploreKind::Song | ExploreKind::Video => Ok(vec![track_of(item)]),
            // An artist isn't a track list; its top songs are the closest thing
            // to one, and they're what YouTube's own play button uses.
            ExploreKind::Artist => self.artist_tracks(item).await,
        }
    }

    /// An artist's playable rows, in the order their page lists them.
    ///
    /// Only the rows: the carousels on an artist page are albums and singles to
    /// *open*, not tracks, and opening each to flatten it would be dozens of
    /// requests for a play button.
    ///
    /// An artist whose page holds no rows at all is an empty queue rather than
    /// an error — there is nothing wrong with the page, it simply has no songs
    /// listed, and a failure here would read as "this artist is broken".
    async fn artist_tracks(&self, item: &ExploreItem) -> Result<Vec<YtTrack>, String> {
        let shelves = self.artist_page(&item.id).await?;
        Ok(shelves
            .iter()
            .flat_map(|shelf| &shelf.items)
            .filter(|i| matches!(i.kind, ExploreKind::Song | ExploreKind::Video))
            .map(|song| YtTrack {
                // The artist's own name stands in where a row didn't link a
                // credit: on their page, an unlinked credit is them.
                artist: song.artist.clone().or_else(|| Some(item.title.clone())),
                playlist_id: item.id.clone(),
                playlist_title: item.title.clone(),
                ..track_of(song)
            })
            .collect())
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
                // Album tracks are songs — YouTube renders them as a still, so
                // there's nothing to watch. It also doesn't report a video type
                // here, and claiming footage we haven't seen would show an
                // enabled toggle that produces a photograph.
                has_video: false,
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

    /// yt-dlp, found once and remembered.
    fn runner(&self) -> Result<ProcessRunner, String> {
        if let Some(bin) = self.yt_dlp_bin.read().ok().and_then(|g| g.clone()) {
            // A binary that moved or was uninstalled between tracks re-detects
            // below rather than failing on a stale path.
            if bin.is_file() {
                return Ok(ProcessRunner::new(bin));
            }
        }
        let runner = ProcessRunner::detect().ok_or_else(|| YtDlpError::NotInstalled.to_string())?;
        if let Ok(mut slot) = self.yt_dlp_bin.write() {
            *slot = Some(runner.bin().to_path_buf());
        }
        Ok(runner)
    }

    /// A still-valid url for `video_id`, if we already paid to resolve one.
    fn cached_target(&self, video_id: &str) -> Option<ytdlp::StreamTarget> {
        cached_in(&self.resolved, video_id)
    }

    /// The cache read every resolve path goes through: a live hit is served
    /// as-is (~µs); a disk-restored hit is probed first — promoted on 200/206,
    /// dropped otherwise so the caller falls through to a fresh resolve.
    fn live_or_probed_target(&self, video_id: &str) -> Option<ytdlp::StreamTarget> {
        if let Some(t) = self.cached_target(video_id) {
            return Some(t);
        }
        let quarantined = self.restored.write().ok()?.remove(video_id)?;
        if probe_ok(&quarantined) {
            // remember_target() also bumps the generation — promotion doesn't
            // change the snapshot union, but the bump is harmless (one spare
            // write).
            self.remember_target(video_id, &quarantined);
            return Some(quarantined);
        }
        // Dead on arrival (new network, revoked url): the union changed, so
        // the saver must write the shrunken truth.
        self.cache_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }

    /// Files a resolved url under its video id, and drops whatever has expired.
    fn remember_target(&self, video_id: &str, target: &ytdlp::StreamTarget) {
        remember_in(&self.resolved, video_id, target);
        self.cache_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// The audio url cache as a JSON envelope, with its generation — or `None`
    /// when there is nothing fresh worth writing.
    ///
    /// The union of the live map and the not-yet-probed restored map (live
    /// wins): snapshotting only the live map would shrink the file to what got
    /// played this session, throwing away restored entries that are still
    /// perfectly probeable tomorrow.
    pub fn url_cache_snapshot(&self) -> Option<(u64, String)> {
        // Read gen before content: a racing write between this and the map
        // reads below just makes the saver run once more (a spare write),
        // rather than mark an unsaved entry as saved.
        let generation = self.url_cache_generation();
        let now = now_secs()?;
        let mut entries: std::collections::HashMap<String, ytdlp::StreamTarget> = self
            .restored
            .read()
            .ok()?
            .iter()
            .filter(|(_, t)| is_fresh(t, now))
            .map(|(k, t)| (k.clone(), t.clone()))
            .collect();
        for (k, t) in self.resolved.read().ok()?.iter() {
            if is_fresh(t, now) {
                entries.insert(k.clone(), t.clone());
            }
        }
        if entries.is_empty() {
            return None;
        }
        let file = UrlCacheFile { version: URL_CACHE_VERSION, entries };
        serde_json::to_string(&file).ok().map(|json| (generation, json))
    }

    /// Load a previous session's url cache into quarantine.
    ///
    /// Tolerant by design — a cache that can't be read is a cache that doesn't
    /// exist, never an error: garbage, an old version, or a clock problem all
    /// just mean starting cold.
    pub fn restore_url_cache(&self, json: &str) {
        let Ok(file) = serde_json::from_str::<UrlCacheFile>(json) else {
            return;
        };
        if file.version != URL_CACHE_VERSION {
            return;
        }
        let Some(now) = now_secs() else { return };
        let Ok(mut restored) = self.restored.write() else {
            return;
        };
        for (id, t) in file.entries {
            if is_fresh(&t, now) {
                restored.insert(id, t);
            }
        }
    }

    pub fn url_cache_generation(&self) -> u64 {
        self.cache_generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Resolves ahead of time, so the next track's url is ready before it plays.
    ///
    /// Same work as [`Self::resolve`] and deliberately no different: it fills the
    /// same cache the play path reads, which is what keeps this an optimisation
    /// rather than a second way to start playback. Errors are the caller's to
    /// ignore — a failed prefetch must cost nothing, because the play path will
    /// resolve again and report the failure properly if it's real.
    pub fn prefetch(&self, video_id: &str) -> Result<(), String> {
        if self.live_or_probed_target(video_id).is_some() {
            return Ok(());
        }
        self.resolve(video_id).map(|_| ())
    }

    /// Warm several tracks' stream urls, one at a time.
    ///
    /// Sequential on purpose: each miss is a full yt-dlp process, and two
    /// spawns visibly contend for the CPU and network the click's own resolve
    /// is using. Cache hits cost nothing ([`Self::prefetch`] checks first), so
    /// a caller can re-send ids freely. Same fire-and-forget contract as
    /// `prefetch`: a failure costs nothing because the play path resolves for
    /// itself and reports properly.
    pub fn prefetch_batch(&self, video_ids: &[String]) {
        for id in video_ids {
            let _ = self.prefetch(id);
        }
    }

    /// Resolve and cache the *video* rendition ahead of time, so opening the
    /// Video tab is instant rather than paying the ~5s yt-dlp spawn on the click.
    ///
    /// Distinct from [`Self::prefetch`], which warms the audio url — a different
    /// resolve into a different cache. [`Self::video_target`] already checks the
    /// cache and fills it, so this is that call with the result discarded.
    pub fn prefetch_video(&self, video_id: &str) -> Result<(), String> {
        self.video_target(video_id).map(|_| ())
    }

    /// Forgets any cached url for `video_id`.
    ///
    /// For the caller that just found one didn't work: a url can die before its
    /// stated expiry (an IP change invalidates it — they're bound to the address
    /// that resolved them), and re-serving it from cache would make that
    /// failure permanent instead of transient.
    pub fn forget(&self, video_id: &str) {
        // Both urls, because the thing that kills one kills the other: they were
        // issued to the same address in the same session. Dropping only the audio
        // would leave the picture replaying a url we already know is dead.
        for cache in [&self.resolved, &self.resolved_video] {
            if let Ok(mut cache) = cache.write() {
                cache.remove(video_id);
            }
        }
        // The union a snapshot would contain just shrank — the saver must
        // notice, or a url we've already flagged dead lingers in the file.
        self.cache_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Resolves a track to a directly-playable `(url, headers)`.
    ///
    /// Sync on purpose: the engine's `StreamResolver` is a sync closure invoked
    /// from the decode worker, and this only spawns a process.
    pub fn stream_target(&self, video_id: &str) -> Result<(String, Vec<(String, String)>), String> {
        let target = self.resolve(video_id)?;
        Ok((target.url, target.headers))
    }

    /// The video-only rendition to show beside the audio, if the track has one.
    ///
    /// Mirrors [`Self::stream_target`]'s `(url, headers)` — the headers matter
    /// as much here: googlevideo checks the User-Agent against the client that
    /// resolved, which is why this can't be handed to a `<video>` element
    /// directly and goes through the loopback proxy instead.
    ///
    /// Uses the same session ordering as audio, and reports failure as an error
    /// for the caller to turn into "no video". Nothing here may affect playback:
    /// the picture is optional, the sound is not.
    pub fn video_target(&self, video_id: &str) -> Result<(String, Vec<(String, String)>), String> {
        use std::sync::atomic::Ordering;
        // Same bargain as the audio path, and for a sharper reason: this resolve
        // is on demand, so its ~5s process start is time the user spends looking
        // at a spinner. Re-opening the Video tab, or re-mounting the element,
        // used to pay it again every time.
        if let Some(hit) = cached_in(&self.resolved_video, video_id) {
            return Ok((hit.url, hit.headers));
        }
        let runner = self.runner()?;
        let cookies = self.cookies_snapshot();
        let file = cookie_file(cookies.as_deref())?;
        let session = file.as_ref().map(CookieFile::path);
        let session_first = self.session_first.load(Ordering::Relaxed);

        let target = ytdlp::resolve_video(
            &runner,
            video_id,
            if session_first { session } else { None },
        )
        .or_else(|first| {
            // Same fallback shape as audio: whichever attempt the session hint
            // didn't lead with is still worth asking.
            match (first, session) {
                (YtDlpError::NoCompatibleFormat(_), Some(s)) if !session_first => {
                    ytdlp::resolve_video(&runner, video_id, Some(s))
                }
                (YtDlpError::NoCompatibleFormat(e), Some(_)) => {
                    let _ = e;
                    ytdlp::resolve_video(&runner, video_id, None)
                }
                (e, _) => Err(e),
            }
        })
        .map_err(|e| e.to_string())?;
        remember_in(&self.resolved_video, video_id, &target);
        Ok((target.url, target.headers))
    }

    /// Like [`Self::stream_target`], but keeps the format details the caller may
    /// want (the container ext is a demuxer hint).
    ///
    /// Falls back to an anonymous resolve when the authenticated one finds no
    /// playable format. The session exists for private and Premium content; the
    /// catalog these tracks come from is public, so a track YouTube won't serve
    /// us an m4a for *while signed in* is still worth asking for signed out —
    /// and measurably answers with itag 140 when asked that way.
    ///
    /// Deliberately narrow: only [`YtDlpError::NoCompatibleFormat`] retries.
    /// "Unavailable" and "Blocked" mean the session is the point, and retrying
    /// those without it would just trade a clear error for a worse one.
    pub fn resolve(&self, video_id: &str) -> Result<ytdlp::StreamTarget, String> {
        use std::sync::atomic::Ordering;
        if let Some(hit) = self.live_or_probed_target(video_id) {
            return Ok(hit);
        }
        let runner = self.runner()?;
        let cookies = self.cookies_snapshot();
        let file = cookie_file(cookies.as_deref())?;
        let resolved = resolve_with_fallback(
            &runner,
            video_id,
            file.as_ref().map(CookieFile::path),
            self.session_first.load(Ordering::Relaxed),
        )?;
        self.remember_target(video_id, &resolved.target);
        // Remember what actually worked, so the next track leads with it. Both
        // directions: if the session starts working again, we go back to it.
        if file.is_some() {
            self.session_first
                .store(resolved.used_session, Ordering::Relaxed);
        }
        Ok(resolved.target)
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

/// A resolved stream, plus which attempt produced it.
#[derive(Debug)]
struct Resolved {
    target: ytdlp::StreamTarget,
    /// True when the session produced it. Lets the caller stop paying for an
    /// attempt that never works — each one is a process spawn and a network
    /// round trip (seconds), directly in front of the play button.
    used_session: bool,
}

/// Resolve `video_id`, trying the session and an anonymous read in whichever
/// order is likelier to work first, and falling back to the other.
///
/// Both attempts always remain available: `session_first: false` is a hint, not
/// a decision. A private or Premium track that anonymous can't see still gets
/// the session — it just isn't first in the queue for it.
///
/// Split out from [`YtMusicState::resolve`] so the *policy* is testable without
/// a real yt-dlp: which failures retry, and in which order, is the part that
/// matters and the part that would quietly rot.
fn resolve_with_fallback(
    runner: &dyn ytdlp::YtDlpRunner,
    video_id: &str,
    session: Option<&Path>,
    session_first: bool,
) -> Result<Resolved, String> {
    // Nothing to alternate with.
    let Some(session) = session else {
        return ytdlp::resolve(runner, video_id, None)
            .map(|target| Resolved {
                target,
                used_session: false,
            })
            .map_err(|e| e.to_string());
    };

    let (first, second) = if session_first {
        (Some(session), None)
    } else {
        (None, Some(session))
    };

    match ytdlp::resolve(runner, video_id, first) {
        Ok(target) => Ok(Resolved {
            target,
            used_session: first.is_some(),
        }),
        // Only a format failure alternates. "Unavailable" and "Blocked" mean the
        // session is the point — dropping it there would trade a clear error for
        // a worse one.
        Err(YtDlpError::NoCompatibleFormat(first_said)) => {
            ytdlp::resolve(runner, video_id, second)
                .map(|target| Resolved {
                    target,
                    used_session: second.is_some(),
                })
                .map_err(|other| {
                    // Both reasons: "it failed twice, differently" is a fact
                    // worth having, and one message would hide the half that
                    // actually explains it.
                    format!("{first_said} (the other attempt also failed: {other})")
                })
        }
        Err(e) => Err(e.to_string()),
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

/// One continuation of a radio: the same `next` POST as the first page, plus
/// the token. Exists for the same reason as [`PlaylistContinuation`] —
/// upstream's `GetContinuationsQuery` can only be built through the typed
/// parser this crate routes around. Wire format is upstream's own
/// (`ctoken` + `continuation`, same header and path).
struct WatchContinuation<'a> {
    base: &'a GetWatchPlaylistQuery<VideoID<'a>>,
    token: String,
}

impl PostQuery for WatchContinuation<'_> {
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

impl ParseFrom<WatchContinuation<'_>> for RawPage {
    fn parse_from(_: ProcessedResult<WatchContinuation<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(RawPage)
    }
}

impl<A: ytmapi_rs::auth::AuthToken> Query<A> for WatchContinuation<'_> {
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

/// The track a song or video row already describes.
///
/// Costs no request: the row states the title, credit, album, running time and
/// video type, which is everything [`YtTrack`] holds. Resolving it to a stream
/// happens later and identically to any other track.
///
/// Callers must only hand this a [`ExploreKind::Song`] or [`ExploreKind::Video`]
/// item — for any other kind `id` is a browse id, and the result would be a
/// track whose video id opens nothing.
fn track_of(item: &ExploreItem) -> YtTrack {
    YtTrack {
        video_id: item.id.clone(),
        title: item.title.clone(),
        artist: item.artist.clone(),
        album: item.album.clone(),
        duration_secs: item.duration_secs,
        thumbnail: item.thumbnail.clone(),
        // A result stands on its own rather than in a playlist. The title is
        // what the library's Folders facet grades it under, and "Songs" or
        // "Videos" is the honest answer to which folder a search result is in.
        playlist_id: item.id.clone(),
        playlist_title: item.title.clone(),
        // Search lists nothing it won't serve; a track that turns out to be
        // blocked fails at resolve time like any other.
        is_available: true,
        has_video: item.has_video,
    }
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

    /// The signed-in session, read once for the whole suite.
    ///
    /// [`YtMusicState::load`] downgrades an unreadable keychain to "signed out",
    /// which is right for the app — it should start regardless — but ruinous
    /// here: every live test below reads "signed out" as "nothing to prove" and
    /// returns green. Measured under `cargo test`'s default parallelism, tests
    /// that pass on their own report "skipping" when run together, so the
    /// keychain does sometimes decline a concurrent read. A suite that reports
    /// success when it in fact ran nothing is worse than no suite.
    ///
    /// So: one read for the binary rather than one per test, and an *error* is
    /// fatal while a genuine absence still skips. "No session" is a reason not
    /// to run; "the keychain wouldn't answer" is a reason not to believe the
    /// result, and the two must not look alike.
    fn live_state() -> Option<&'static YtMusicState> {
        static STATE: std::sync::OnceLock<Option<YtMusicState>> = std::sync::OnceLock::new();
        STATE
            .get_or_init(|| match cookies::load() {
                Err(e) => panic!(
                    "the keychain would not answer, so no live test here can be \
                     trusted either way: {e}"
                ),
                Ok(None) => None,
                Ok(Some(_)) => {
                    let state = YtMusicState::load();
                    state.signed_in().then_some(state)
                }
            })
            .as_ref()
    }

    fn target(expires_at: Option<u64>) -> ytdlp::StreamTarget {
        ytdlp::StreamTarget {
            url: "https://rr2---sn-abc.googlevideo.com/videoplayback?itag=140".into(),
            headers: vec![("User-Agent".into(), "Mozilla/5.0".into())],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: Some(130),
            expires_at,
        }
    }

    const NOW: u64 = 1_784_290_000;

    #[test]
    fn a_url_with_hours_left_is_fresh() {
        // googlevideo issues these ~6h out.
        assert!(is_fresh(&target(Some(NOW + 6 * 3600)), NOW));
    }

    #[test]
    fn a_url_inside_the_margin_is_not_fresh() {
        // Still valid, but not for long enough to play a track through.
        assert!(!is_fresh(&target(Some(NOW + 60)), NOW));
        assert!(!is_fresh(&target(Some(NOW + EXPIRY_MARGIN_SECS)), NOW));
    }

    #[test]
    fn an_expired_url_is_not_fresh() {
        assert!(!is_fresh(&target(Some(NOW - 1)), NOW));
        // Already past: must not wrap into a huge remaining lifetime.
        assert!(!is_fresh(&target(Some(1)), NOW));
    }

    /// A url we can't date can't be re-served: a guessed lifetime is how a cache
    /// starts handing out dead links.
    #[test]
    fn a_url_without_a_stated_expiry_is_never_fresh() {
        assert!(!is_fresh(&target(None), NOW));
    }

    #[test]
    fn a_remembered_url_comes_back() {
        let state = YtMusicState::new();
        state.remember_target("vid1", &target(Some(now_secs().unwrap() + 6 * 3600)));
        assert_eq!(state.cached_target("vid1").map(|t| t.format_id), Some("140".into()));
        assert!(state.cached_target("other").is_none());
    }

    #[test]
    fn an_undatable_url_is_not_remembered() {
        let state = YtMusicState::new();
        state.remember_target("vid1", &target(None));
        assert!(state.cached_target("vid1").is_none());
    }

    #[test]
    fn a_url_that_is_already_stale_is_not_served() {
        let state = YtMusicState::new();
        state.remember_target("vid1", &target(Some(now_secs().unwrap() + 30)));
        assert!(state.cached_target("vid1").is_none());
    }

    /// A link can die before its stated deadline — googlevideo binds each to the
    /// address that resolved it, and the url says nothing about that. Whoever
    /// finds one broken has to be able to drop it, or the cache would replay it
    /// on every retry and make a transient failure permanent.
    #[test]
    fn a_url_can_be_forgotten_before_it_expires() {
        let state = YtMusicState::new();
        state.remember_target("vid1", &target(Some(now_secs().unwrap() + 6 * 3600)));
        assert!(state.cached_target("vid1").is_some());
        state.forget("vid1");
        assert!(state.cached_target("vid1").is_none());
    }

    fn video_target_fixture(expires_at: Option<u64>) -> ytdlp::StreamTarget {
        ytdlp::StreamTarget {
            url: "https://rr2---sn-abc.googlevideo.com/videoplayback?itag=137".into(),
            headers: vec![("User-Agent".into(), "Mozilla/5.0".into())],
            ext: "mp4".into(),
            format_id: "137".into(),
            abr_kbps: None,
            expires_at,
        }
    }

    /// One video id resolves to two different urls — the m4a the engine decodes
    /// and the picture-only rendition. A single map keyed by id would hand the
    /// `<video>` element an audio url (or the engine a silent one), which is a
    /// far worse failure than the cache miss it was meant to avoid.
    #[test]
    fn the_audio_and_video_caches_do_not_answer_for_each_other() {
        let state = YtMusicState::new();
        let live = now_secs().unwrap() + 6 * 3600;
        state.remember_target("vid1", &target(Some(live)));
        remember_in(&state.resolved_video, "vid1", &video_target_fixture(Some(live)));

        assert_eq!(state.cached_target("vid1").map(|t| t.format_id), Some("140".into()));
        assert_eq!(
            cached_in(&state.resolved_video, "vid1").map(|t| t.format_id),
            Some("137".into()),
        );
    }

    /// A dead url is dead in both directions: they were issued to the same
    /// address in the same session, so whatever invalidated one invalidated both.
    #[test]
    fn forgetting_drops_the_video_url_too() {
        let state = YtMusicState::new();
        let live = now_secs().unwrap() + 6 * 3600;
        state.remember_target("vid1", &target(Some(live)));
        remember_in(&state.resolved_video, "vid1", &video_target_fixture(Some(live)));

        state.forget("vid1");

        assert!(state.cached_target("vid1").is_none());
        assert!(
            cached_in(&state.resolved_video, "vid1").is_none(),
            "a video url must not outlive the forget that dropped its audio"
        );
    }

    /// The video cache is bound by the same freshness rule as the audio one —
    /// it shares the code, and this is what says that on purpose.
    #[test]
    fn the_video_cache_will_not_serve_a_stale_url() {
        let state = YtMusicState::new();
        remember_in(
            &state.resolved_video,
            "vid1",
            &video_target_fixture(Some(now_secs().unwrap() + 30)),
        );
        assert!(cached_in(&state.resolved_video, "vid1").is_none());

        remember_in(&state.resolved_video, "vid2", &video_target_fixture(None));
        assert!(cached_in(&state.resolved_video, "vid2").is_none());
    }

    /// Entries are only useful while live, so what makes one stale also makes it
    /// collectable — that's the whole eviction policy.
    #[test]
    fn writing_drops_whatever_has_expired() {
        let state = YtMusicState::new();
        let now = now_secs().unwrap();
        {
            let mut c = state.resolved.write().unwrap();
            c.insert("dead".into(), target(Some(now - 10)));
        }
        state.remember_target("live", &target(Some(now + 6 * 3600)));
        let cache = state.resolved.read().unwrap();
        assert!(!cache.contains_key("dead"), "an expired entry must not survive a write");
        assert!(cache.contains_key("live"));
    }

    /// The gap between two tracks, measured.
    ///
    /// A cold resolve is a yt-dlp process start — Python up, extractors imported,
    /// then the network — and lands around five seconds. That is the whole reason
    /// this cache exists, and the only honest way to know it works is to time
    /// both halves against the real binary.
    #[test]
    #[ignore = "requires yt-dlp on PATH and network access"]
    fn a_second_resolve_costs_nothing() {
        const ID: &str = "dQw4w9WgXcQ";
        let state = YtMusicState::new();

        let t0 = std::time::Instant::now();
        let Ok(cold_target) = state.resolve(ID) else {
            eprintln!("skipping: yt-dlp could not resolve (no binary, or no network)");
            return;
        };
        let cold = t0.elapsed();

        let t1 = std::time::Instant::now();
        let warm_target = state.resolve(ID).expect("a cached resolve cannot fail");
        let warm = t1.elapsed();

        eprintln!("cold {cold:?}, warm {warm:?}");
        assert_eq!(warm_target.url, cold_target.url, "the cache must answer with what it stored");
        assert!(
            warm < std::time::Duration::from_millis(50),
            "a warm resolve must not spawn a process: took {warm:?}"
        );
        assert!(
            cold_target.expires_at.is_some(),
            "googlevideo stamps expire= on every url; without it nothing is cacheable"
        );

        // Dropping it puts the cost back — proof the speed came from the cache
        // and not from yt-dlp having warmed up.
        state.forget(ID);
        let t2 = std::time::Instant::now();
        let _ = state.resolve(ID);
        assert!(
            t2.elapsed() > warm * 10,
            "forgetting must actually force a re-resolve"
        );
    }

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

    /* ---- the anonymous-resolve fallback ---- */

    /// A runner that answers differently depending on whether the call carried
    /// `--cookies` — which is exactly the asymmetry the fallback exists for.
    struct SessionSensitive {
        signed_in: Result<String, YtDlpError>,
        anonymous: Result<String, YtDlpError>,
        /// Every invocation, so a test can assert the retry really dropped it.
        calls: std::sync::Mutex<Vec<bool>>,
    }

    impl ytdlp::YtDlpRunner for SessionSensitive {
        fn run(&self, args: &[String]) -> Result<String, YtDlpError> {
            let had_cookies = args.iter().any(|a| a == "--cookies");
            self.calls.lock().unwrap().push(had_cookies);
            if had_cookies {
                self.signed_in.clone()
            } else {
                self.anonymous.clone()
            }
        }
    }

    fn ok_stdout() -> String {
        [
            "https://rr2---sn-abc.googlevideo.com/videoplayback?itag=140",
            "m4a",
            "140",
            "129.639",
            "{}",
        ]
        .join("\n")
    }

    fn session_path() -> &'static Path {
        Path::new("/tmp/hm-test-cookies.txt")
    }

    /// The case observed in the wild: signed in, YouTube says no format; signed
    /// out, the same video hands over itag 140.
    #[test]
    fn retries_without_the_session_when_signed_in_finds_no_format() {
        let runner = SessionSensitive {
            signed_in: Err(YtDlpError::NoCompatibleFormat("Requested format is not available".into())),
            anonymous: Ok(ok_stdout()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let got = resolve_with_fallback(&runner, "syc4SzrubKY", Some(session_path()), true)
            .expect("the anonymous retry should succeed");
        assert_eq!(got.target.format_id, "140");
        assert!(!got.used_session, "the anonymous attempt is what worked");
        // Tried with the session first, then without — in that order.
        assert_eq!(*runner.calls.lock().unwrap(), [true, false]);
    }

    /// The session is the whole point for a blocked track; retrying without it
    /// would replace a clear, actionable error with a worse one.
    #[test]
    fn does_not_drop_the_session_for_a_blocked_track() {
        let runner = SessionSensitive {
            signed_in: Err(YtDlpError::Blocked("sign in to confirm you're not a bot".into())),
            anonymous: Ok(ok_stdout()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let err = resolve_with_fallback(&runner, "vid", Some(session_path()), true).unwrap_err();
        assert!(err.contains("blocked"), "got {err:?}");
        assert_eq!(*runner.calls.lock().unwrap(), [true], "must not retry");
    }

    #[test]
    fn does_not_drop_the_session_for_an_unavailable_track() {
        let runner = SessionSensitive {
            signed_in: Err(YtDlpError::Unavailable("Video unavailable".into())),
            anonymous: Ok(ok_stdout()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        assert!(resolve_with_fallback(&runner, "vid", Some(session_path()), true).is_err());
        assert_eq!(*runner.calls.lock().unwrap(), [true], "must not retry");
    }

    /// Signed out already — there's nothing to fall back to, so one attempt.
    #[test]
    fn does_not_retry_when_there_was_no_session() {
        let runner = SessionSensitive {
            signed_in: Ok(ok_stdout()),
            anonymous: Err(YtDlpError::NoCompatibleFormat("nope".into())),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        assert!(resolve_with_fallback(&runner, "vid", None, true).is_err());
        assert_eq!(*runner.calls.lock().unwrap(), [false]);
    }

    /// The point of the whole hint: once the session is known not to yield
    /// formats, don't spend a process spawn and a network round trip on it
    /// before every single track.
    #[test]
    fn a_known_bad_session_is_not_tried_first() {
        let runner = SessionSensitive {
            signed_in: Err(YtDlpError::NoCompatibleFormat("no format".into())),
            anonymous: Ok(ok_stdout()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let got = resolve_with_fallback(&runner, "vid", Some(session_path()), false)
            .expect("anonymous works");
        assert_eq!(got.target.format_id, "140");
        // One call, without cookies. No doomed attempt in front of it.
        assert_eq!(*runner.calls.lock().unwrap(), [false]);
    }

    /// …but the session is still there for anything anonymous can't see, so a
    /// private track doesn't become unplayable just because public ones taught
    /// us to lead with anonymous.
    #[test]
    fn a_private_track_still_reaches_the_session_when_anonymous_leads() {
        let runner = SessionSensitive {
            signed_in: Ok(ok_stdout()),
            anonymous: Err(YtDlpError::NoCompatibleFormat("not visible signed out".into())),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let got = resolve_with_fallback(&runner, "vid", Some(session_path()), false)
            .expect("the session should still be reached");
        assert!(got.used_session);
        assert_eq!(*runner.calls.lock().unwrap(), [false, true]);
    }

    /// Both failed: say so, and keep both reasons — the second is usually the
    /// one that explains the first.
    #[test]
    fn reports_both_attempts_when_the_retry_also_fails() {
        let runner = SessionSensitive {
            signed_in: Err(YtDlpError::NoCompatibleFormat("signed-in reason".into())),
            anonymous: Err(YtDlpError::NoCompatibleFormat("anonymous reason".into())),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let err = resolve_with_fallback(&runner, "vid", Some(session_path()), true).unwrap_err();
        assert!(err.contains("signed-in reason"), "got {err:?}");
        assert!(err.contains("anonymous reason"), "got {err:?}");
        assert_eq!(*runner.calls.lock().unwrap(), [true, false]);
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
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let sections = state.explore_categories().await.expect("categories must load");
        let total: usize = sections.iter().map(|s| s.categories.len()).sum();
        assert!(total > 0, "Explore should offer categories");

        let mut empty = Vec::new();
        let mut seen = KindTally::default();
        // Genre pages lead with a Songs shelf and carry a Music videos one;
        // neither is a browsable card, so both are the shelves most easily lost.
        let mut with_songs = 0;
        let mut with_videos = 0;
        for section in &sections {
            for cat in &section.categories {
                let shelves = explore_page_insisting(state, &cat.params, &cat.title).await;
                if shelves.is_empty() {
                    empty.push(cat.title.clone());
                    continue;
                }
                let mut songs_here = 0;
                let mut videos_here = 0;
                for shelf in &shelves {
                    for item in &shelf.items {
                        seen.count(item.kind);
                        match item.kind {
                            ExploreKind::Song => songs_here += 1,
                            ExploreKind::Video => videos_here += 1,
                            _ => {}
                        }
                    }
                }
                with_songs += usize::from(songs_here > 0);
                with_videos += usize::from(videos_here > 0);
            }
        }
        eprintln!(
            "{total} categories: {seen:?}; {with_songs} with songs, \
             {with_videos} with videos; {} empty {empty:?}",
            empty.len()
        );
        assert!(
            empty.is_empty(),
            "every category should yield at least one shelf, these didn't: {empty:?}"
        );
        assert!(
            seen.albums > 0,
            "Explore should surface albums — that's the point"
        );
        // The Songs shelf is a carousel of list rows rather than of cards, and a
        // reader that only knows cards drops all ~50 of them without a trace.
        assert!(
            with_songs > 0,
            "genre pages lead with a Songs shelf; none was read"
        );
        assert!(
            with_videos > 0,
            "genre pages carry a Music videos shelf; none was read"
        );
    }

    /// One category page, asked for twice before being believed.
    ///
    /// The sweep is 44 requests and shares an account with every other live test
    /// beside it; under that load YouTube sometimes cuts a response short
    /// ("error decoding response body"). That is a fact about the network, not
    /// about the parser, and failing the sweep for it would report a bug that
    /// isn't there — while never failing would hide a real outage. Asking again
    /// separates the two: once is noise, twice is an answer.
    async fn explore_page_insisting(
        state: &YtMusicState,
        params: &str,
        title: &str,
    ) -> Vec<ExploreShelf> {
        match state.explore_page(params).await {
            Ok(shelves) => shelves,
            Err(first) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                state
                    .explore_page(params)
                    .await
                    .unwrap_or_else(|second| panic!("\"{title}\" failed twice: {first}; {second}"))
            }
        }
    }

    /// Every kind an Explore or search page can offer, counted.
    #[derive(Default, Debug)]
    struct KindTally {
        playlists: usize,
        albums: usize,
        artists: usize,
        songs: usize,
        videos: usize,
    }

    impl KindTally {
        fn count(&mut self, kind: ExploreKind) {
            let slot = match kind {
                ExploreKind::Playlist => &mut self.playlists,
                ExploreKind::Album => &mut self.albums,
                ExploreKind::Artist => &mut self.artists,
                ExploreKind::Song => &mut self.songs,
                ExploreKind::Video => &mut self.videos,
            };
            *slot += 1;
        }

        fn total(&self) -> usize {
            self.playlists + self.albums + self.artists + self.songs + self.videos
        }
    }

    /// Opening an item must produce queueable tracks — for both kinds. Uses the
    /// first album and first playlist Explore actually offers today.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_explore_items_open_into_tracks() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
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
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
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

    /* ---- search, live ---- */

    /// A query every filter has real answers for.
    const LIVE_QUERY: &str = "burna boy";

    /// Each filter must return results, and of the kind it was asked for.
    ///
    /// Unit tests cannot catch what this catches: upstream's own parsers pass
    /// their tests and still return nothing, because the fixtures were written
    /// from the same belief as the parser. Only YouTube can say what YouTube
    /// sends.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_search_answers_every_filter() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        // What each filter must actually produce. "top" is excluded from the
        // kind check on purpose: it's YouTube's own mix and may hold anything.
        let expected: &[(&str, Option<ExploreKind>)] = &[
            ("songs", Some(ExploreKind::Song)),
            ("videos", Some(ExploreKind::Video)),
            ("albums", Some(ExploreKind::Album)),
            ("artists", Some(ExploreKind::Artist)),
            ("playlists", Some(ExploreKind::Playlist)),
            ("top", None),
        ];

        for (filter, want) in expected {
            let shelves = state
                .search(LIVE_QUERY, filter)
                .await
                .unwrap_or_else(|e| panic!("search({filter:?}) must not error: {e}"));
            let mut tally = KindTally::default();
            for shelf in &shelves {
                for item in &shelf.items {
                    tally.count(item.kind);
                }
            }
            eprintln!(
                "{filter:>10}: {} shelves {:?} -> {tally:?}",
                shelves.len(),
                shelves.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            );
            assert!(
                tally.total() > 0,
                "search({filter:?}) returned nothing — an empty result for a query \
                 this well known is the signature of a parser that matches no rows"
            );
            if let Some(want) = want {
                let got = match want {
                    ExploreKind::Song => tally.songs,
                    ExploreKind::Video => tally.videos,
                    ExploreKind::Album => tally.albums,
                    ExploreKind::Artist => tally.artists,
                    ExploreKind::Playlist => tally.playlists,
                };
                assert!(got > 0, "search({filter:?}) produced no {want:?}: {tally:?}");
            }
            // Whatever the kind, every item must be renderable and openable.
            for item in shelves.iter().flat_map(|s| &s.items) {
                assert!(!item.id.is_empty(), "an item with no id opens nothing");
                assert!(!item.title.is_empty(), "an item with no title renders blank");
            }
        }
    }

    /// A song result must arrive with the detail YT Music shows beside it — the
    /// credit, the album and the running time — since a row that states them and
    /// a parser that drops them look identical from the outside.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_search_songs_carry_their_detail() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let shelves = state.search(LIVE_QUERY, "songs").await.expect("songs");
        let songs: Vec<&ExploreItem> = shelves
            .iter()
            .flat_map(|s| &s.items)
            .filter(|i| i.kind == ExploreKind::Song)
            .collect();
        assert!(!songs.is_empty(), "there must be songs to inspect");

        let with_artist = songs.iter().filter(|s| s.artist.is_some()).count();
        let with_album = songs.iter().filter(|s| s.album.is_some()).count();
        let with_duration = songs.iter().filter(|s| s.duration_secs.is_some()).count();
        eprintln!(
            "{} songs: {with_artist} credited, {with_album} with an album, \
             {with_duration} timed",
            songs.len()
        );
        for s in songs.iter().take(3) {
            eprintln!(
                "  {:?} — {:?} / {:?} / {:?}s",
                s.title, s.artist, s.album, s.duration_secs
            );
        }
        // Not "all": a row may legitimately credit someone unlinkable, or be a
        // single with no album. A *zero* is the tell — that's a pointer that
        // never matches, which is how `has_video` shipped false for 1192 tracks.
        assert!(with_artist > 0, "no song was credited — the artist link is not being read");
        assert!(with_album > 0, "no song had an album — the album link is not being read");
        assert!(with_duration > 0, "no song was timed — the duration run is not being read");
    }

    /// The VL trap, end to end: a playlist found by search must open into tracks.
    ///
    /// Measured live, the bare `PL…` id answers HTTP 400 while the `VL`-prefixed
    /// one returns rows, so this is the assertion that a search result carries an
    /// id the rest of the app can actually use.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_a_playlist_from_search_opens_into_tracks() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let shelves = state.search(LIVE_QUERY, "playlists").await.expect("playlists");
        let playlist = shelves
            .iter()
            .flat_map(|s| &s.items)
            .find(|i| i.kind == ExploreKind::Playlist)
            .expect("a playlist result");
        assert!(
            playlist.id.starts_with("VL"),
            "a playlist id must reach the browse query VL-prefixed, got {:?}",
            playlist.id
        );
        let tracks = state
            .explore_tracks(playlist)
            .await
            .unwrap_or_else(|e| panic!("\"{}\" must open: {e}", playlist.title));
        eprintln!("playlist {:?} -> {} tracks", playlist.title, tracks.len());
        assert!(!tracks.is_empty(), "a playlist from search should have tracks");
    }

    /// An artist result must open into a page with songs on it, and those songs
    /// must be playable without a further request.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_an_artist_from_search_opens_into_a_page_with_tracks() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let shelves = state.search(LIVE_QUERY, "artists").await.expect("artists");
        let artist = shelves
            .iter()
            .flat_map(|s| &s.items)
            .find(|i| i.kind == ExploreKind::Artist)
            .expect("an artist result")
            .clone();
        eprintln!("artist {:?} ({})", artist.title, artist.id);

        let page = state
            .artist_page(&artist.id)
            .await
            .unwrap_or_else(|e| panic!("the artist page must load: {e}"));
        let mut tally = KindTally::default();
        for item in page.iter().flat_map(|s| &s.items) {
            tally.count(item.kind);
        }
        eprintln!(
            "  {} shelves {:?} -> {tally:?}",
            page.len(),
            page.iter().map(|s| s.title.as_str()).collect::<Vec<_>>()
        );
        assert!(!page.is_empty(), "an artist page should have shelves");
        assert!(
            tally.songs > 0,
            "an artist page leads with a Top songs list shelf; none was read"
        );
        assert!(
            tally.albums > 0,
            "an artist page carries album carousels; none was read"
        );

        // And the play button: opening the artist queues their songs.
        let tracks = state
            .explore_tracks(&artist)
            .await
            .expect("an artist must open into tracks");
        eprintln!("  play-all -> {} tracks", tracks.len());
        assert!(!tracks.is_empty(), "an artist should queue their top songs");
        assert!(
            tracks.iter().all(|t| !t.video_id.is_empty()),
            "every queued track needs a video id to be playable"
        );
    }

    /// A song result must be playable straight from the row, with no second
    /// request — the whole reason a row carries what it carries.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_a_song_from_search_opens_into_itself() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let shelves = state.search(LIVE_QUERY, "songs").await.expect("songs");
        let song = shelves
            .iter()
            .flat_map(|s| &s.items)
            .find(|i| i.kind == ExploreKind::Song)
            .expect("a song result");
        let tracks = state.explore_tracks(song).await.expect("a song must open");
        assert_eq!(tracks.len(), 1, "a song is one track");
        assert_eq!(tracks[0].video_id, song.id);
        assert_eq!(tracks[0].title, song.title);
        assert!(!tracks[0].has_video, "a song is an audio entity — no footage");
        eprintln!("song {:?} -> {:?}", song.title, tracks[0]);
    }

    /// Suggestions must never raise: this fires per keystroke.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_search_suggestions_complete_a_partial_query() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let got = state.search_suggestions("burna").await;
        eprintln!("suggestions: {got:?}");
        assert!(!got.is_empty(), "a well-known prefix should suggest something");
        assert!(
            got.iter().all(|s| !s.trim().is_empty()),
            "a blank suggestion is a half-read run"
        );
        // Nothing typed asks YouTube nothing.
        assert!(state.search_suggestions("   ").await.is_empty());
    }

    /// Run with `cargo test -p hm-ytmusic -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_library_listing_parses() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        let playlists = state.playlists().await.expect("the listing must parse");
        eprintln!("parsed {} playlists", playlists.len());
        assert!(!playlists.is_empty(), "a signed-in library should list something");
    }

    /// Run with `cargo test -p hm-ytmusic -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a signed-in YT Music session in the keychain and network access"]
    async fn live_radio_pages_endlessly() {
        let Some(state) = live_state() else {
            eprintln!("skipping: no session in the keychain — sign in through the app first");
            return;
        };
        // A track with a famously deep similarity graph.
        let seed = "dQw4w9WgXcQ";
        let first = state.radio(seed).await.expect("the radio page must parse");
        eprintln!("radio page 1: {} tracks", first.tracks.len());
        assert!(
            first.tracks.len() >= 10,
            "a radio should fill a queue, got {}",
            first.tracks.len()
        );
        assert!(
            first.tracks.iter().all(|t| t.video_id != seed),
            "the seed must not be re-queued"
        );
        let token = first.continuation.expect("radio pages must chain");
        let second = state
            .radio_continue(seed, &token)
            .await
            .expect("the continuation must parse");
        eprintln!("radio page 2: {} tracks", second.tracks.len());
        assert!(!second.tracks.is_empty(), "the continuation should keep the radio going");
        assert!(second.continuation.is_some(), "radio should always offer another page");
    }

    // NOTE: named `fresh_in` rather than the brief's `target` — this module
    // already has a `target(expires_at: Option<u64>)` fixture (used above,
    // ~line 1248) and Rust free functions can't be overloaded by signature.
    fn fresh_in(expires_in: i64) -> ytdlp::StreamTarget {
        let now = now_secs().unwrap() as i64;
        ytdlp::StreamTarget {
            url: "https://cdn.example/x".into(),
            headers: vec![("User-Agent".into(), "hm".into())],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: Some(129),
            expires_at: (now + expires_in > 0).then(|| (now + expires_in) as u64),
        }
    }

    /// The whole point of persistence: what one session remembers, the next
    /// session can restore — and still-fresh means fresh on BOTH trips.
    #[test]
    fn url_cache_round_trips_fresh_entries() {
        let a = YtMusicState::new();
        a.remember_target("vid1", &fresh_in(6 * 3600));
        a.remember_target("vid2", &fresh_in(60)); // inside EXPIRY_MARGIN — not fresh
        let (_, json) = a.url_cache_snapshot().expect("one fresh entry to save");

        let b = YtMusicState::new();
        b.restore_url_cache(&json);
        let restored = b.restored.read().unwrap();
        assert!(restored.contains_key("vid1"), "the fresh entry must round-trip");
        assert!(!restored.contains_key("vid2"), "a near-expiry entry is not worth restoring");
        // Restored entries are quarantined, not live: the play path must probe
        // them first (IP-bound urls), so cached_target must NOT serve them.
        drop(restored);
        assert!(b.cached_target("vid1").is_none());
    }

    #[test]
    fn snapshot_is_the_union_of_live_and_restored() {
        let a = YtMusicState::new();
        a.remember_target("live1", &fresh_in(6 * 3600));
        let (_, json) = a.url_cache_snapshot().unwrap();

        let b = YtMusicState::new();
        b.restore_url_cache(&json);
        b.remember_target("live2", &fresh_in(6 * 3600));
        let (_, json2) = b.url_cache_snapshot().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&json2).unwrap();
        let entries = envelope.pointer("/entries").unwrap().as_object().unwrap();
        // Without the union, every relaunch would shrink the file to only
        // what got played that session.
        assert!(entries.contains_key("live1"), "an unprobed restored entry must persist");
        assert!(entries.contains_key("live2"));
    }

    #[test]
    fn garbage_and_wrong_versions_are_ignored_not_fatal() {
        let s = YtMusicState::new();
        s.restore_url_cache("not json at all");
        s.restore_url_cache("{\"version\": 99, \"entries\": {}}");
        s.restore_url_cache("{}");
        assert!(s.restored.read().unwrap().is_empty());
        assert!(s.url_cache_snapshot().is_none(), "nothing restorable means nothing to save");
    }

    /// The saver polls the generation to skip no-op writes.
    #[test]
    fn generation_moves_on_remember_not_on_read_or_restore() {
        let s = YtMusicState::new();
        let g0 = s.url_cache_generation();
        let _ = s.cached_target("vid1");
        assert_eq!(s.url_cache_generation(), g0, "a read must not dirty the cache");
        s.remember_target("vid1", &fresh_in(6 * 3600));
        let g1 = s.url_cache_generation();
        assert!(g1 > g0, "a write must dirty the cache");
        let (_, json) = s.url_cache_snapshot().unwrap();
        let t = YtMusicState::new();
        t.restore_url_cache(&json);
        assert_eq!(
            t.url_cache_generation(),
            0,
            "restoring what came FROM the file must not schedule a rewrite of it"
        );
    }

    /// One request, canned response, captured request lines.
    fn one_shot_server(
        status_line: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut lines = Vec::new();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
                lines.push(line.trim_end().to_string());
            }
            let mut w = stream;
            let _ = write!(w, "{status_line}\r\nContent-Length: 2\r\n\r\nok");
            lines
        });
        (format!("http://{addr}/probe"), handle)
    }

    fn restored_state_with(url: String) -> YtMusicState {
        let s = YtMusicState::new();
        let now = now_secs().unwrap();
        let t = ytdlp::StreamTarget {
            url,
            headers: vec![("User-Agent".into(), "hm-probe-test".into())],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: None,
            expires_at: Some(now + 6 * 3600),
        };
        s.restored.write().unwrap().insert("vid1".into(), t);
        s
    }

    /// A restored entry that answers is promoted: served now, live (unprobed)
    /// forever after — and the probe itself asks for two bytes, not the track.
    #[test]
    fn a_healthy_restored_entry_is_probed_once_then_live() {
        let (url, server) = one_shot_server("HTTP/1.1 206 Partial Content");
        let s = restored_state_with(url.clone());
        let got = s.live_or_probed_target("vid1").expect("a 206 probe must serve the entry");
        assert_eq!(got.url, url);
        let lines = server.join().unwrap();
        assert!(
            lines.iter().any(|l| l.eq_ignore_ascii_case("range: bytes=0-1")),
            "the probe must ask for two bytes, not the body; got {lines:#?}"
        );
        assert!(
            lines.iter().any(|l| l.eq_ignore_ascii_case("user-agent: hm-probe-test")),
            "the entry's own headers must go out — the CDN checks them"
        );
        // Promoted: second read is a live hit, no second request (the one-shot
        // server is already gone, so a re-probe would return None here).
        assert_eq!(s.live_or_probed_target("vid1").unwrap().url, got.url);
        assert!(s.restored.read().unwrap().is_empty(), "quarantine is over");
    }

    /// 403 is what an IP-bound url looks like from a new network: the entry is
    /// dead on arrival — drop it so the caller falls through to a fresh
    /// resolve, and dirty the cache so the dead entry leaves the file too.
    #[test]
    fn a_dead_restored_entry_is_dropped_and_dirties_the_cache() {
        let (url, server) = one_shot_server("HTTP/1.1 403 Forbidden");
        let s = restored_state_with(url);
        let g0 = s.url_cache_generation();
        assert!(s.live_or_probed_target("vid1").is_none());
        let _ = server.join();
        assert!(s.restored.read().unwrap().is_empty());
        assert!(s.cached_target("vid1").is_none());
        assert!(s.url_cache_generation() > g0, "the union changed; the saver must notice");
    }

    /// Same-session entries never probe — the whole point of quarantining.
    #[test]
    fn a_live_entry_is_served_without_any_request() {
        let s = YtMusicState::new();
        let now = now_secs().unwrap();
        let t = ytdlp::StreamTarget {
            url: "http://127.0.0.1:1/unreachable".into(),
            headers: vec![],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: None,
            expires_at: Some(now + 6 * 3600),
        };
        s.remember_target("vid1", &t);
        assert_eq!(
            s.live_or_probed_target("vid1").expect("live hits must not probe").url,
            t.url
        );
    }

    /// `forget()` must dirty the cache too — it removes from `resolved`, and if
    /// the generation doesn't move, a dead entry that already made it into a
    /// prior snapshot would never be written out of the file.
    #[test]
    fn forgetting_a_url_dirties_the_cache() {
        let s = YtMusicState::new();
        s.remember_target("vid1", &fresh_in(6 * 3600));
        let g0 = s.url_cache_generation();
        s.forget("vid1");
        assert!(s.url_cache_generation() > g0, "forgetting changes the union; the saver must notice");
    }
}
