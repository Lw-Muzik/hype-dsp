//! Movies — full-length films from **verified rights-holder YouTube
//! distributor channels** (FilmRise, Shout! Studios, Maverick Movies,
//! Mosfilm), enumerated **keylessly** via the app's bundled yt-dlp
//! `--flat-playlist` discovery. Sibling to `commands::movies` (Internet
//! Archive): that module is the "Classics" shelf, this one is "modern".
//!
//! **Legal boundary, load-bearing:** this module only ever *lists* a
//! channel's uploads (id/title/duration) — it never resolves or proxies a
//! movie's actual video/audio stream. Playback for every title this command
//! returns goes through the official YouTube iframe embed
//! (`YoutubeMoviePlayer.tsx`), which is the only path that keeps this
//! feature inside YouTube's ToS. The yt-dlp `--flat-playlist`
//! stream-*extraction* machinery the app uses for its own music
//! (`hm_ytmusic::ytdlp::resolve`) is deliberately never called from here.
//!
//! Reuses the existing yt-dlp binary resolution
//! (`hm_ytmusic::ytdlp::ProcessRunner::detect`, managed-dir-wins-PATH) and its
//! `CREATE_NO_WINDOW`-guarded spawn rather than inventing a second yt-dlp
//! launcher — see `ytdlp.rs`'s own doc comment on `ProcessRunner::command`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hm_core::IpcError;
use hm_ytmusic::ytdlp::{ProcessRunner, YtDlpRunner};
use serde::{Deserialize, Serialize};

/// Full movies run well over an hour; the trailers/clips that share a
/// distributor's channel run one to a few minutes. 60 minutes cleanly
/// separates the two without excluding any real film among the spec's
/// confirmed channels.
const MIN_MOVIE_DURATION_SECS: f64 = 3600.0;

/// A looser floor that still counts as a movie when the uploader's own title
/// says so — catches the rare edit that runs a little under an hour without
/// pulling in ordinary trailers (which never claim to be the full movie).
const MIN_TITLED_DURATION_SECS: f64 = 2400.0;

/// How many *candidate* entries to pull from the channel before filtering.
/// yt-dlp's flat-playlist listing is cheap (no per-video metadata fetch), but
/// some of these channels have uploaded thousands of clips over the years —
/// this keeps one discovery call bounded rather than walking a channel's
/// entire history to find enough full movies.
const DISCOVERY_FETCH_CAP: u32 = 200;

/// How long a channel's listing is trusted before a fresh discovery is worth
/// another yt-dlp spawn. These catalogs turn over on the order of weeks, not
/// minutes, so a session-scoped cache saves the (multi-second) spawn on every
/// visit to the Movies screen without ever serving something stale-feeling.
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

fn thumbnail_url(video_id: &str) -> String {
    format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg")
}

fn channel_videos_url(channel_id: &str) -> String {
    format!("https://www.youtube.com/channel/{channel_id}/videos")
}

/* ------------------------------------------------------------------ IPC shape */

/// One full movie enumerated from a distributor channel — enough to render a
/// shelf card and, on play, hand the video id straight to the iframe embed.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct YtMovie {
    pub video_id: String,
    pub title: String,
    pub duration_sec: u64,
    pub thumbnail: String,
    pub channel_title: Option<String>,
}

/* --------------------------------------------------- yt-dlp flat-playlist DTOs */

/// `yt-dlp --flat-playlist -J <channel videos url>` output. Only the fields
/// this module reads — flat-playlist entries carry a great deal more that
/// isn't needed here.
#[derive(Deserialize, Default)]
struct FlatPlaylistResponse {
    #[serde(default)]
    entries: Vec<FlatEntry>,
    /// The channel's display name, when yt-dlp reports one at the playlist
    /// level (it usually does for a `/channel/<id>/videos` listing).
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
}

#[derive(Deserialize)]
struct FlatEntry {
    id: String,
    #[serde(default)]
    title: Option<String>,
    /// Absent for a handful of entry kinds (e.g. an unfinished livestream) —
    /// treated as "can't tell if it's a movie", not zero.
    #[serde(default, deserialize_with = "flexible_opt_duration")]
    duration: Option<f64>,
}

/// yt-dlp documents `duration` as numeric, but flat-playlist output has been
/// seen to stringify it on some entry kinds — tolerate either rather than
/// dropping the whole entry (the same defensive pattern `commands::movies`
/// uses for Archive's `year`/`downloads`).
fn flexible_opt_duration<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(v.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }))
}

/// Strips a marketing suffix a distributor tacks on after the real title —
/// e.g. `"Night of the Comet | FULL MOVIE | FilmRise Movies"` becomes
/// `"Night of the Comet"`. Only ever trims from the first `|` onward: the
/// part before it is trusted verbatim, since some real titles legitimately
/// contain other punctuation (colons, dashes) this deliberately leaves alone.
fn clean_title(raw: &str) -> String {
    raw.split('|').next().unwrap_or(raw).trim().to_string()
}

/// Whether an entry's title claims to be the whole film — the loosened
/// duration floor ([`MIN_TITLED_DURATION_SECS`]) only ever applies alongside
/// this, so a short clip can't qualify just by running a few seconds longer
/// than a trailer.
fn title_claims_full_movie(title: &str) -> bool {
    title.to_lowercase().contains("full movie")
}

/// Filters and maps one flat-playlist entry. `None` for anything that reads
/// like a trailer/clip rather than a full movie, or that yt-dlp reported no
/// duration for at all (nothing to judge it by).
fn entry_to_movie(entry: FlatEntry, channel_title: Option<&str>) -> Option<YtMovie> {
    let duration = entry.duration?;
    let title_raw = entry
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| entry.id.clone());

    let is_movie_length = duration >= MIN_MOVIE_DURATION_SECS
        || (duration >= MIN_TITLED_DURATION_SECS && title_claims_full_movie(&title_raw));
    if !is_movie_length {
        return None;
    }

    Some(YtMovie {
        thumbnail: thumbnail_url(&entry.id),
        video_id: entry.id,
        title: clean_title(&title_raw),
        duration_sec: duration.round().max(0.0) as u64,
        channel_title: channel_title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

/// Runs the discovery listing and maps it down to full-length movies, most
/// recently uploaded first (yt-dlp's own channel order).
fn discover(runner: &dyn YtDlpRunner, channel_id: &str) -> Result<Vec<YtMovie>, IpcError> {
    let args = vec![
        "--flat-playlist".to_string(),
        "--no-warnings".to_string(),
        "--playlist-end".to_string(),
        DISCOVERY_FETCH_CAP.to_string(),
        "-J".to_string(),
        channel_videos_url(channel_id),
    ];
    let stdout = runner
        .run(&args)
        .map_err(|e| IpcError::new("ytdlp", e.to_string()))?;
    let parsed: FlatPlaylistResponse = serde_json::from_str(&stdout)
        .map_err(|e| IpcError::new("parse", format!("Unexpected channel listing: {e}")))?;

    let channel_title = parsed.channel.as_deref().or(parsed.uploader.as_deref());
    Ok(parsed
        .entries
        .into_iter()
        .filter_map(|e| entry_to_movie(e, channel_title))
        .collect())
}

/* --------------------------------------------------------------- session cache */

struct CacheEntry {
    movies: Vec<YtMovie>,
    fetched_at: Instant,
}

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, CacheEntry>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/* ----------------------------------------------------------------- commands */

/// Enumerate one distributor channel's full-length uploads, discovered
/// keylessly via yt-dlp and filtered to movie-length items (see
/// [`entry_to_movie`]). Cached in-memory per channel for [`CACHE_TTL`] — a
/// channel's catalog doesn't turn over minute to minute, and this yt-dlp
/// spawn is the slowest thing the Movies screen does.
///
/// Errors (yt-dlp missing, network, an unparsable listing) surface as an
/// [`IpcError`] and the calling shelf degrades gracefully rather than taking
/// the rest of the Movies screen down with it — the same contract
/// `movies_browse` gives its own shelves.
// `(async)`: shells out to yt-dlp and waits on the network, same as every
// other yt-dlp-backed command in this app.
#[tauri::command(async)]
pub fn yt_movies_channel(channel_id: String, limit: u32) -> Result<Vec<YtMovie>, IpcError> {
    let channel_id = channel_id.trim().to_string();
    if channel_id.is_empty() {
        return Err(IpcError::new("invalid", "channelId is required"));
    }
    let limit = limit.clamp(1, DISCOVERY_FETCH_CAP) as usize;

    if let Some(entry) = cache().lock().unwrap().get(&channel_id) {
        if entry.fetched_at.elapsed() < CACHE_TTL {
            let mut movies = entry.movies.clone();
            movies.truncate(limit);
            return Ok(movies);
        }
    }

    let runner = ProcessRunner::detect().ok_or_else(|| {
        IpcError::new(
            "not_installed",
            "yt-dlp isn't installed — it's needed to list this channel's movies.",
        )
    })?;

    let movies = discover(&runner, &channel_id)?;
    cache().lock().unwrap().insert(
        channel_id,
        CacheEntry {
            movies: movies.clone(),
            fetched_at: Instant::now(),
        },
    );

    let mut result = movies;
    result.truncate(limit);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, title: &str, duration: f64) -> FlatEntry {
        FlatEntry {
            id: id.to_string(),
            title: Some(title.to_string()),
            duration: Some(duration),
        }
    }

    #[test]
    fn keeps_only_hour_plus_entries() {
        let movie = entry_to_movie(entry("a", "A Real Feature", 5460.0), Some("FilmRise"));
        assert!(movie.is_some());
        let trailer = entry_to_movie(entry("b", "Official Trailer", 105.0), Some("FilmRise"));
        assert!(trailer.is_none());
        let clip = entry_to_movie(entry("c", "Behind the Scenes Clip", 42.0), Some("FilmRise"));
        assert!(clip.is_none());
    }

    #[test]
    fn accepts_a_shorter_edit_only_when_the_title_says_full_movie() {
        let titled = entry_to_movie(entry("a", "A Movie | FULL MOVIE | FilmRise", 2500.0), None);
        assert!(titled.is_some(), "a titled full movie under an hour should still count");

        let untitled_same_length = entry_to_movie(entry("b", "A Random Upload", 2500.0), None);
        assert!(
            untitled_same_length.is_none(),
            "the same length without the title claim must not qualify"
        );
    }

    #[test]
    fn missing_duration_is_excluded_not_zero() {
        let e = FlatEntry { id: "x".to_string(), title: Some("No Duration".to_string()), duration: None };
        assert!(entry_to_movie(e, None).is_none());
    }

    #[test]
    fn tolerates_duration_sent_as_a_string() {
        let json = r#"{ "id": "z", "title": "Quirky Feature", "duration": "3721" }"#;
        let parsed: FlatEntry = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.duration, Some(3721.0));
        assert!(entry_to_movie(parsed, None).is_some());
    }

    #[test]
    fn strips_marketing_suffix_but_keeps_punctuation_before_it() {
        assert_eq!(
            clean_title("Night of the Comet | FULL MOVIE | FilmRise Movies"),
            "Night of the Comet"
        );
        assert_eq!(clean_title("Plain Title, No Suffix"), "Plain Title, No Suffix");
        assert_eq!(clean_title("  Trim Me  | Extra"), "Trim Me");
    }

    #[test]
    fn builds_the_documented_thumbnail_shape() {
        assert_eq!(
            thumbnail_url("dQw4w9WgXcQ"),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"
        );
    }

    #[test]
    fn maps_a_full_channel_listing_fixture() {
        let json = r#"{
            "channel": "FilmRise Movies",
            "entries": [
                { "id": "abc123", "title": "Night of the Comet | FULL MOVIE | FilmRise Movies", "duration": 5460 },
                { "id": "def456", "title": "Trailer: Night of the Comet", "duration": 105 },
                { "id": "ghi789", "title": "Another Feature Film", "duration": "3721" },
                { "id": "jkl000", "title": "Clip: behind the scenes", "duration": 42 },
                { "id": "mno111", "title": "Livestream replay" }
            ]
        }"#;
        let parsed: FlatPlaylistResponse = serde_json::from_str(json).unwrap();
        let channel_title = parsed.channel.as_deref().or(parsed.uploader.as_deref());
        let movies: Vec<YtMovie> = parsed
            .entries
            .into_iter()
            .filter_map(|e| entry_to_movie(e, channel_title))
            .collect();

        assert_eq!(movies.len(), 2, "only the two hour-plus entries should survive: {movies:?}");
        assert_eq!(movies[0].video_id, "abc123");
        assert_eq!(movies[0].title, "Night of the Comet");
        assert_eq!(movies[0].duration_sec, 5460);
        assert_eq!(movies[0].channel_title.as_deref(), Some("FilmRise Movies"));
        assert_eq!(movies[1].video_id, "ghi789");
        assert_eq!(movies[1].duration_sec, 3721);
    }

    #[test]
    fn falls_back_to_uploader_when_channel_is_absent() {
        let json = r#"{
            "uploader": "Mosfilm",
            "entries": [ { "id": "a", "title": "A Feature", "duration": 4000 } ]
        }"#;
        let parsed: FlatPlaylistResponse = serde_json::from_str(json).unwrap();
        let channel_title = parsed.channel.as_deref().or(parsed.uploader.as_deref());
        let movies: Vec<YtMovie> = parsed
            .entries
            .into_iter()
            .filter_map(|e| entry_to_movie(e, channel_title))
            .collect();
        assert_eq!(movies[0].channel_title.as_deref(), Some("Mosfilm"));
    }

    #[test]
    fn empty_id_is_rejected_before_any_yt_dlp_spawn() {
        let err = yt_movies_channel("   ".to_string(), 24).unwrap_err();
        assert_eq!(err.code, "invalid");
    }

    #[test]
    fn serialises_camel_case_for_the_frontend() {
        let movie = YtMovie {
            video_id: "abc".to_string(),
            title: "A Film".to_string(),
            duration_sec: 5400,
            thumbnail: "https://i.ytimg.com/vi/abc/hqdefault.jpg".to_string(),
            channel_title: Some("FilmRise".to_string()),
        };
        let json = serde_json::to_string(&movie).unwrap();
        assert!(json.contains("\"videoId\":\"abc\""));
        assert!(json.contains("\"durationSec\":5400"));
        assert!(json.contains("\"channelTitle\":\"FilmRise\""));
    }
}
