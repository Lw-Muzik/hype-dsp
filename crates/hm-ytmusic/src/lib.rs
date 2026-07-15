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
pub mod ytdlp;

use cookies::{CookieFile, YtCookie};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use ytdlp::{ProcessRunner, YtDlpError};
use ytmapi_rs::auth::BrowserToken;
use ytmapi_rs::common::{PlaylistID, YoutubeID};
use ytmapi_rs::parse::PlaylistItem;
use ytmapi_rs::YtMusic;

/// How many playlists to fetch tracks for at once. Enough to hide latency on a
/// large library without hammering the API into rate-limiting us.
const PLAYLIST_FETCH_CONCURRENCY: usize = 6;

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
    pub async fn playlists(&self) -> Result<Vec<YtPlaylist>, String> {
        let yt = self.client().await?;
        let raw = yt
            .get_library_playlists()
            .await
            .map_err(|e| format!("Could not load playlists: {e}"))?;
        Ok(raw.into_iter().map(map_playlist).collect())
    }

    /// One playlist's tracks.
    pub async fn playlist_tracks(&self, playlist: &YtPlaylist) -> Result<Vec<YtTrack>, String> {
        let yt = self.client().await?;
        fetch_tracks(&yt, playlist).await
    }

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

async fn fetch_tracks(
    yt: &YtMusic<BrowserToken>,
    playlist: &YtPlaylist,
) -> Result<Vec<YtTrack>, String> {
    let items = yt
        .get_playlist_tracks(PlaylistID::from_raw(playlist.id.clone()))
        .await
        .map_err(|e| format!("Could not load \"{}\": {e}", playlist.title))?;
    Ok(items
        .into_iter()
        .filter_map(|item| map_item(item, playlist))
        .collect())
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

/// Maps one playlist entry.
///
/// Only songs and videos are music. Episodes (podcasts) and library uploads are
/// dropped: uploads have no `video_id` to resolve, and podcasts aren't what this
/// view is for. Matched exhaustively on purpose — if ytmapi-rs grows a variant,
/// this should fail the build rather than quietly drop tracks.
fn map_item(item: PlaylistItem, playlist: &YtPlaylist) -> Option<YtTrack> {
    match item {
        PlaylistItem::Song(s) => Some(YtTrack {
            video_id: s.video_id.get_raw().to_string(),
            title: s.title,
            artist: join_artists(&s.artists),
            album: (!s.album.name.is_empty()).then_some(s.album.name),
            duration_secs: parse_duration(&s.duration),
            thumbnail: best_thumbnail(&s.thumbnails),
            playlist_id: playlist.id.clone(),
            playlist_title: playlist.title.clone(),
            is_available: s.is_available,
        }),
        PlaylistItem::Video(v) => Some(YtTrack {
            video_id: v.video_id.get_raw().to_string(),
            title: v.title,
            artist: (!v.channel_name.is_empty()).then_some(v.channel_name),
            album: None,
            duration_secs: parse_duration(&v.duration),
            thumbnail: best_thumbnail(&v.thumbnails),
            playlist_id: playlist.id.clone(),
            playlist_title: playlist.title.clone(),
            is_available: v.is_available,
        }),
        PlaylistItem::Episode(_) | PlaylistItem::UploadSong(_) => None,
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
fn parse_duration(raw: &str) -> Option<f64> {
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
}
