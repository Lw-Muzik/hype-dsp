//! YouTube Music commands: sign in, list playlists, play, download.
//!
//! Playback deliberately adds no new machinery. `stream_target` yields the same
//! `(url, headers)` the cloud path does, so YT Music tracks reach the engine
//! through `play_stream` / `StreamQueueSource` — Range seeking, resume-on-drop
//! and the gapless queue all included.
//!
//! The queue still resolves lazily rather than up front, but for the ordering,
//! not for freshness: a url is good for hours (see `hm_ytmusic`), and resolving
//! the whole queue before the first note would put every track's yt-dlp spawn in
//! front of the play button. The resolver asks for a fresh url only when a retry
//! says the last one didn't work.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hm_audio::stream_queue::{StreamResolver, StreamTarget};
use hm_audio::AudioEngine;
use hm_core::IpcError;
use hm_link::LinkState;
use hm_ytmusic::cookies::{self, YtCookie};
use hm_ytmusic::explore::{ExploreItem, ExploreShelf};
use hm_ytmusic::{ExploreSection, RadioBatch, YtMusicState, YtMusicStatus, YtPlaylist, YtTrack};

use crate::tv_proxy::TvProxy;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::ytmusic::{CachedLibrary, YtLibraryCache, YtSettings};

const LOGIN_WINDOW: &str = "ytmusic-login";
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
/// Long enough for 2FA and a password manager, short enough that an abandoned
/// window doesn't poll forever.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const LOGIN_POLL: Duration = Duration::from_millis(700);

/// User-agent for the sign-in window.
///
/// Tauri's webview is not Chromium outside Windows, and the engines it does use
/// send a UA that music.youtube.com rejects with a "not optimized for your
/// browser — get Chrome" wall instead of a login form:
///
/// * **macOS** — WKWebView omits the `Version/<n>` token that Safari sends, so
///   YouTube can't identify it. Claiming Safari is honest here (WKWebView *is*
///   Safari's engine) and YT Music supports Safari, so this asks for the page
///   it would already serve us.
/// * **Linux** — WebKitGTK is likewise unrecognised, and a Safari UA would be a
///   lie (there is no Safari on Linux), so it claims Chrome.
/// * **Windows** — WebView2 is Chromium and is recognised as-is; overriding it
///   could only make things worse.
#[cfg(target_os = "macos")]
const LOGIN_UA: Option<&str> = Some(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
);
#[cfg(target_os = "linux")]
const LOGIN_UA: Option<&str> = Some(
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
);
#[cfg(target_os = "windows")]
const LOGIN_UA: Option<&str> = None;

/// A library listing plus whether it came from the on-disk cache — same contract
/// as `CloudAudioPage`, so the front end can render instantly and refresh behind.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtMusicPage {
    pub playlists: Vec<YtPlaylist>,
    pub tracks: Vec<YtTrack>,
    pub from_cache: bool,
}

/// Download progress, emitted on `ytmusic:download`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub video_id: String,
    /// `fetching` (yt-dlp → laptop), `sending` (laptop → phone), `done`, `error`.
    pub phase: String,
    pub bytes: u64,
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl DownloadProgress {
    fn new(video_id: &str, phase: &str) -> Self {
        Self {
            video_id: video_id.to_string(),
            phase: phase.to_string(),
            bytes: 0,
            total: None,
            message: None,
        }
    }
}

/* ---- auth ---- */

/// Whether we're signed in, and whether yt-dlp/ffmpeg are around.
// `(async)`: probing yt-dlp shells out for `--version`.
#[tauri::command(async)]
pub fn ytmusic_status(state: State<'_, YtMusicState>) -> YtMusicStatus {
    state.status()
}

/// Open a window on Google's sign-in and wait for a YouTube session to appear.
///
/// We poll the cookie jar rather than watch for a redirect: the login flow can
/// bounce through consent, 2FA and account-picker pages, and any URL we matched
/// on would be a guess about Google's routing. The cookies are the thing we
/// actually need, so waiting for *them* is both simpler and more robust.
#[tauri::command]
pub async fn ytmusic_sign_in(
    app: AppHandle,
    state: State<'_, YtMusicState>,
) -> Result<YtMusicStatus, IpcError> {
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = existing.set_focus();
    } else {
        let mut builder = WebviewWindowBuilder::new(
            &app,
            LOGIN_WINDOW,
            WebviewUrl::External(LOGIN_URL.parse().map_err(|e| {
                IpcError::new("ytmusic", format!("bad sign-in URL: {e}"))
            })?),
        )
        .title("Sign in to YouTube Music")
        .inner_size(520.0, 760.0);
        if let Some(ua) = LOGIN_UA {
            builder = builder.user_agent(ua);
        }
        builder.build().map_err(|e| {
            IpcError::new("ytmusic", format!("couldn't open the sign-in window: {e}"))
        })?;
    }

    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        tokio::time::sleep(LOGIN_POLL).await;

        // Gone means the user closed it — that's a cancel, not a failure.
        let Some(win) = app.get_webview_window(LOGIN_WINDOW) else {
            return Err(IpcError::new("cancelled", "Sign-in was cancelled."));
        };
        if Instant::now() > deadline {
            let _ = win.close();
            return Err(IpcError::new(
                "timeout",
                "Sign-in timed out. Please try again.",
            ));
        }

        let captured = harvest_cookies(&win);
        if cookies::is_signed_in(&captured) {
            let _ = win.close();
            state
                .sign_in(captured)
                .await
                .map_err(|e| IpcError::new("ytmusic", e))?;
            return Ok(state.status());
        }
    }
}

/// Forget the session and drop the cached listing, so signing in as someone else
/// can't show the previous account's playlists.
#[tauri::command]
pub async fn ytmusic_sign_out(
    state: State<'_, YtMusicState>,
    cache: State<'_, YtLibraryCache>,
) -> Result<(), IpcError> {
    state
        .sign_out()
        .await
        .map_err(|e| IpcError::new("ytmusic", e))?;
    cache.clear();
    Ok(())
}

/// Reads the session out of the login window's cookie jar.
///
/// Both domains are needed: the session lives on `.youtube.com`, but `SAPISID` —
/// which the API's auth hash is derived from — is set on `.google.com`. Taking
/// only one yields a client that looks signed in and returns nothing.
///
/// This takes the whole jar and filters it here, rather than asking for the
/// cookies of each host with `cookies_for_url`. That looks like the obvious API
/// for the job and is a trap: it matches with `cookie.domain() == url.domain()`,
/// and `domain()` has already had its leading dot stripped — so a `.youtube.com`
/// cookie reports `youtube.com` and never equals `music.youtube.com`. Every
/// cookie that carries the session is a domain cookie, so *all* of them were
/// silently dropped and sign-in could never complete. Owning the match keeps the
/// rule visible, tested (`cookies::wanted_domain`), and ours to reason about.
fn harvest_cookies(win: &WebviewWindow) -> Vec<YtCookie> {
    let Ok(found) = win.cookies() else {
        return Vec::new();
    };
    let mut out: Vec<YtCookie> = Vec::new();
    for c in found {
        // Filter on the cookie's own domain, before `map_cookie` massages it:
        // this jar holds every host the login flow touched, and a cookie we
        // can't attribute must not be assumed to be YouTube's.
        if !c.domain().is_some_and(cookies::wanted_domain) {
            continue;
        }
        let mapped = map_cookie(&c);
        // A name can legitimately appear on both domains; keeping exact
        // duplicates would double it up in the cookies.txt we hand yt-dlp.
        if !out
            .iter()
            .any(|e| e.name == mapped.name && e.domain == mapped.domain)
        {
            out.push(mapped);
        }
    }
    // The jar arrives in no defined order, and `cookies::header` keeps the first
    // of a duplicated name — so without this, which `SID` the API sees would be
    // luck. Stable, so it only reorders across domains.
    out.sort_by_key(|c| cookies::domain_rank(&c.domain));
    out
}

fn map_cookie(c: &tauri::webview::Cookie<'_>) -> YtCookie {
    // The `cookie` crate strips a domain's leading dot, but Netscape format uses
    // it to mean "match subdomains" — and every cookie that matters here is a
    // domain cookie on `.youtube.com` / `.google.com`. Put it back, or yt-dlp
    // won't match them against `music.youtube.com`.
    let domain = c
        .domain()
        .map(|d| {
            if d.starts_with('.') {
                d.to_string()
            } else {
                format!(".{d}")
            }
        })
        .unwrap_or_else(|| ".youtube.com".to_string());
    YtCookie {
        name: c.name().to_string(),
        value: c.value().to_string(),
        domain,
        path: c.path().unwrap_or("/").to_string(),
        expires: c.expires_datetime().map(|d| d.unix_timestamp()),
        secure: c.secure().unwrap_or(false),
        http_only: c.http_only().unwrap_or(false),
    }
}

/* ---- library ---- */

/// Every track across the user's playlists.
///
/// `refresh: false` serves the cache when there is one, so opening the app is
/// instant; the front end then calls again with `true` in the background.
#[tauri::command]
pub async fn ytmusic_all_tracks(
    state: State<'_, YtMusicState>,
    cache: State<'_, YtLibraryCache>,
    refresh: bool,
) -> Result<YtMusicPage, IpcError> {
    if !refresh {
        if let Some(cached) = cache.get() {
            return Ok(YtMusicPage {
                playlists: cached.playlists,
                tracks: cached.tracks,
                from_cache: true,
            });
        }
    }
    let (playlists, tracks) = state
        .all_tracks()
        .await
        .map_err(|e| IpcError::new("ytmusic", e))?;
    cache.put(CachedLibrary {
        playlists: playlists.clone(),
        tracks: tracks.clone(),
    });
    Ok(YtMusicPage {
        playlists,
        tracks,
        from_cache: false,
    })
}

/* ---- explore ---- */

/// The mood/genre categories YouTube offers.
#[tauri::command]
pub async fn ytmusic_explore_categories(
    state: State<'_, YtMusicState>,
) -> Result<Vec<ExploreSection>, IpcError> {
    state
        .explore_categories()
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// One category's shelves of playlists and albums.
///
/// Uncached on purpose: Explore is YouTube's live catalog, and being current is
/// the whole reason to browse it rather than merge it into the library.
#[tauri::command]
pub async fn ytmusic_explore_page(
    state: State<'_, YtMusicState>,
    params: String,
) -> Result<Vec<ExploreShelf>, IpcError> {
    state
        .explore_page(&params)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// The tracks behind one Explore item, ready to queue.
#[tauri::command]
pub async fn ytmusic_explore_tracks(
    state: State<'_, YtMusicState>,
    item: ExploreItem,
) -> Result<Vec<YtTrack>, IpcError> {
    state
        .explore_tracks(&item)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/* ---- search ---- */

/// Searching YouTube's catalog.
///
/// `filter` is one of `songs`, `videos`, `albums`, `artists`, `playlists`, or
/// anything else for YouTube's own mix of all of them. Uncached for the same
/// reason as Explore: results are only worth having current.
#[tauri::command]
pub async fn ytmusic_search(
    state: State<'_, YtMusicState>,
    query: String,
    filter: String,
) -> Result<Vec<ExploreShelf>, IpcError> {
    state
        .search(&query, &filter)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// What YouTube would complete a half-typed query with.
///
/// Cannot fail: this fires on every keystroke, and a type-ahead that can raise
/// an error dialog is worse than one that occasionally offers nothing.
#[tauri::command]
pub async fn ytmusic_search_suggestions(
    state: State<'_, YtMusicState>,
    query: String,
) -> Result<Vec<String>, IpcError> {
    Ok(state.search_suggestions(&query).await)
}

/// An artist's page — top songs, albums, singles and videos, as YouTube ordered
/// them.
#[tauri::command]
pub async fn ytmusic_artist_page(
    state: State<'_, YtMusicState>,
    browse_id: String,
) -> Result<Vec<ExploreShelf>, IpcError> {
    state
        .artist_page(&browse_id)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// The endless "up next" YT Music derives from one song — its radio. Returns
/// the first page (~25–50 similar tracks) and the token for the next one.
#[tauri::command]
pub async fn ytmusic_radio(
    state: State<'_, YtMusicState>,
    video_id: String,
) -> Result<RadioBatch, IpcError> {
    state
        .radio(&video_id)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// The next page of a radio. `video_id` is the seed the radio was started
/// from — the wire format re-POSTs the full body plus the token.
#[tauri::command]
pub async fn ytmusic_radio_continue(
    state: State<'_, YtMusicState>,
    video_id: String,
    token: String,
) -> Result<RadioBatch, IpcError> {
    state
        .radio_continue(&video_id, &token)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/* ---- video ---- */

/// A loopback URL for this track's video-only rendition, for a muted `<video>`.
///
/// Video-only and muted is what keeps the enhancement chain in the path: the
/// rendition has no audio track to play, so the element is a picture and nothing
/// else, while the engine goes on doing the actual work. The proxy exists
/// because the element can reach neither googlevideo's origin (CSP) nor its
/// User-Agent requirement.
///
/// Resolving costs a yt-dlp spawn, so the front end asks only when the user
/// turns video on. A failure here is "no video", never a playback error —
/// nothing about the picture may touch the sound.
// `(async)`: shells out to yt-dlp and waits on the network.
#[tauri::command(async)]
pub fn ytmusic_video_url(
    state: State<'_, YtMusicState>,
    proxy: State<'_, TvProxy>,
    video_id: String,
) -> Result<String, IpcError> {
    let (url, headers) = state
        .video_target(&video_id)
        .map_err(|e| IpcError::new("ytmusic", e))?;
    let ua = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v.as_str());
    Ok(proxy.video_url(&url, ua))
}

/* ---- playback ---- */

/// Resolve a track's url ahead of playing it.
///
/// The gap between two tracks *is* the resolve: a yt-dlp process start, a Python
/// interpreter and extractor import before the network is even touched, ~5s. On
/// the single-track path that work could only begin once the previous track had
/// ended, so all of it landed in silence. Asking early moves it under the track
/// already playing, leaving only the two-second prebuffer to hear.
///
/// Best-effort by contract. The answer is a cache entry, not a return value, and
/// the play path resolves for itself regardless — so a prefetch that fails, or
/// races the user skipping, costs nothing and must never surface an error.
// `(async)`: shells out to yt-dlp and waits on the network.
#[tauri::command(async)]
pub fn ytmusic_prefetch(state: State<'_, YtMusicState>, video_id: String) {
    let _ = state.prefetch(&video_id);
}

/// Warm the *video* rendition's url so the Video tab opens instantly. Same
/// fire-and-forget contract as [`ytmusic_prefetch`]: a failure or a race with a
/// skip costs nothing, because the Video tab resolves for itself regardless.
// `(async)`: shells out to yt-dlp and waits on the network.
#[tauri::command(async)]
pub fn ytmusic_video_prefetch(state: State<'_, YtMusicState>, video_id: String) {
    let _ = state.prefetch_video(&video_id);
}

/// Resolve a track and play it through the chain.
// `(async)`: resolving shells out to yt-dlp and waits on the network.
#[tauri::command(async)]
pub fn ytmusic_play(
    state: State<'_, YtMusicState>,
    engine: State<'_, AudioEngine>,
    video_id: String,
    duration_secs: Option<f64>,
) -> Result<(), IpcError> {
    let (url, headers) = state
        .stream_target(&video_id)
        .map_err(|e| IpcError::new("ytmusic", e))?;
    // Unlike cloud files, we have a duration from the API — pass it so the seek
    // bar is right from the first frame instead of waiting on the container.
    engine
        .play_stream(url, headers, duration_secs)
        .map_err(Into::into)
}

/// One track in a YT Music gapless/crossfade queue.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YtQueueItem {
    pub video_id: String,
}

/// Play a queue of YT Music tracks gaplessly / crossfading.
///
/// Each URL is resolved lazily, one track ahead. That isn't just an optimisation
/// here: a googlevideo URL carries an `expire=` stamp and is bound to the IP that
/// resolved it, so URLs resolved for the whole queue up front would be stale (or
/// invalid on a network change) by the time playback reached them.
#[tauri::command]
pub fn player_play_ytmusic_queue(
    app: AppHandle,
    engine: State<'_, AudioEngine>,
    items: Vec<YtQueueItem>,
    start: usize,
) -> Result<(), IpcError> {
    if items.is_empty() {
        return Err(IpcError::new("invalid", "empty YouTube Music queue"));
    }
    let count = items.len();
    let items = Arc::new(items);
    let resolver: StreamResolver = Arc::new(move |i: usize, fresh: bool| {
        let item = items
            .get(i)
            .ok_or_else(|| "queue index out of range".to_string())?;
        let state = app.state::<YtMusicState>();
        // The previous url didn't work, so it must not be answered with again —
        // it may have been bound to an address we no longer have.
        if fresh {
            state.forget(&item.video_id);
        }
        let (url, headers) = state.stream_target(&item.video_id)?;
        Ok(StreamTarget {
            url,
            headers,
            // Pinned by the format selector in `hm-ytmusic` — the engine can't
            // decode the Opus rendition YouTube would otherwise prefer.
            ext: Some("m4a".to_string()),
        })
    });
    engine
        .play_stream_queue(resolver, count, start)
        .map_err(Into::into)
}

/* ---- downloads ---- */

/// The folder downloads go to.
#[tauri::command]
pub fn ytmusic_download_dir(settings: State<'_, YtSettings>) -> String {
    settings.download_dir().to_string_lossy().into_owned()
}

/// Set the download folder; pass `None`/empty to reset to the default.
#[tauri::command]
pub fn ytmusic_set_download_dir(settings: State<'_, YtSettings>, dir: Option<String>) -> String {
    settings.set_download_dir(dir);
    settings.download_dir().to_string_lossy().into_owned()
}

/// Download a track to the laptop and index it into the library.
///
/// Indexing is the point: once it's a local track it seeks properly, plays
/// offline, and keeps working if yt-dlp breaks later. The download stops being
/// a YouTube Music thing and becomes a file the user owns.
// `(async)`: yt-dlp download — minutes, not milliseconds.
#[tauri::command(async)]
pub fn ytmusic_download(
    app: AppHandle,
    state: State<'_, YtMusicState>,
    settings: State<'_, YtSettings>,
    store: State<'_, hm_core::MediaStore>,
    video_id: String,
) -> Result<String, IpcError> {
    let dir = settings.download_dir();
    let path = fetch_to_disk(&app, &state, &dir, &video_id)?;

    crate::commands::library::index_paths(&app, &store, std::slice::from_ref(&path));

    emit(&app, DownloadProgress::new(&video_id, "done"));
    Ok(path.to_string_lossy().into_owned())
}

/// Download a track and send it to a paired phone.
///
/// Two plain phases rather than a pipe from yt-dlp straight into the upload:
/// each is independently retryable, the file is complete (and verifiable) before
/// it's sent, and the laptop keeps a copy. It also means the upload leg is just
/// "send a local file", which is why [`hm_link::LinkState::upload`] takes a path
/// and works for any local track, not only YouTube Music ones.
// `(async)`: download + upload, both long and blocking.
#[tauri::command(async)]
pub fn ytmusic_download_to_phone(
    app: AppHandle,
    state: State<'_, YtMusicState>,
    settings: State<'_, YtSettings>,
    store: State<'_, hm_core::MediaStore>,
    link: State<'_, LinkState>,
    video_id: String,
    device_id: String,
) -> Result<(), IpcError> {
    let dir = settings.download_dir();
    let path = fetch_to_disk(&app, &state, &dir, &video_id)?;

    // The laptop copy is a real download, so let the library have it too.
    crate::commands::library::index_paths(&app, &store, std::slice::from_ref(&path));

    let progress_app = app.clone();
    let progress_id = video_id.clone();
    link.upload(&device_id, &path, move |sent, total| {
        emit(
            &progress_app,
            DownloadProgress {
                video_id: progress_id.clone(),
                phase: "sending".into(),
                bytes: sent,
                total: Some(total),
                message: None,
            },
        );
    })
    .map_err(|e| {
        emit_error(&app, &video_id, &e);
        IpcError::new("link", e)
    })?;

    emit(&app, DownloadProgress::new(&video_id, "done"));
    Ok(())
}

/// Shared first leg: yt-dlp writes the track into `dir`, reporting progress.
fn fetch_to_disk(
    app: &AppHandle,
    state: &YtMusicState,
    dir: &std::path::Path,
    video_id: &str,
) -> Result<PathBuf, IpcError> {
    emit(app, DownloadProgress::new(video_id, "fetching"));

    let progress_app = app.clone();
    let progress_id = video_id.to_string();
    let path = state
        .download(video_id, dir, move |p| {
            emit(
                &progress_app,
                DownloadProgress {
                    video_id: progress_id.clone(),
                    phase: "fetching".into(),
                    bytes: p.downloaded_bytes,
                    total: p.total_bytes,
                    message: None,
                },
            );
        })
        .map_err(|e| {
            emit_error(app, video_id, &e);
            IpcError::new("ytmusic", e)
        })?;

    // yt-dlp is told to write inside `dir`, but it also picks the filename from
    // track metadata. Verify rather than trust: a title can be anything.
    if !crate::ytmusic::is_within(dir, &path) {
        let msg = "the download landed outside the downloads folder".to_string();
        emit_error(app, video_id, &msg);
        return Err(IpcError::new("ytmusic", msg));
    }
    Ok(path)
}

fn emit(app: &AppHandle, p: DownloadProgress) {
    let _ = app.emit("ytmusic:download", p);
}

fn emit_error(app: &AppHandle, video_id: &str, message: &str) {
    emit(
        app,
        DownloadProgress {
            video_id: video_id.to_string(),
            phase: "error".into(),
            bytes: 0,
            total: None,
            message: Some(message.to_string()),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_serialises_camel_case_for_the_frontend() {
        let json = serde_json::to_string(&DownloadProgress {
            video_id: "abc".into(),
            phase: "fetching".into(),
            bytes: 10,
            total: Some(20),
            message: None,
        })
        .unwrap();
        assert!(json.contains("\"videoId\":\"abc\""));
        // A null message would make the UI render an empty error line.
        assert!(!json.contains("message"));
    }
}
