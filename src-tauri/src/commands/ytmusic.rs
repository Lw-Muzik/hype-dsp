//! YouTube Music commands: sign in, list playlists, play, download.
//!
//! Playback deliberately adds no new machinery. `stream_target` yields the same
//! `(url, headers)` the cloud path does, so YT Music tracks reach the engine
//! through `play_stream` / `StreamQueueSource` — Range seeking, resume-on-drop
//! and the gapless queue all included. Like Dropbox's temporary links (and more
//! so: they carry `expire=` and are pinned to the resolving IP), the URLs are
//! short-lived, so the queue resolver re-resolves per attempt rather than
//! resolving the queue up front.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hm_audio::stream_queue::{StreamResolver, StreamTarget};
use hm_audio::AudioEngine;
use hm_core::IpcError;
use hm_link::LinkState;
use hm_ytmusic::cookies::{self, YtCookie};
use hm_ytmusic::{YtMusicState, YtMusicStatus, YtPlaylist, YtTrack};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::ytmusic::{CachedLibrary, YtLibraryCache, YtSettings};

const LOGIN_WINDOW: &str = "ytmusic-login";
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
/// Long enough for 2FA and a password manager, short enough that an abandoned
/// window doesn't poll forever.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const LOGIN_POLL: Duration = Duration::from_millis(700);

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
        WebviewWindowBuilder::new(
            &app,
            LOGIN_WINDOW,
            WebviewUrl::External(LOGIN_URL.parse().map_err(|e| {
                IpcError::new("ytmusic", format!("bad sign-in URL: {e}"))
            })?),
        )
        .title("Sign in to YouTube Music")
        .inner_size(520.0, 760.0)
        .build()
        .map_err(|e| IpcError::new("ytmusic", format!("couldn't open the sign-in window: {e}")))?;
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
/// Both hosts are needed: the session lives on `.youtube.com`, but `SAPISID` —
/// which the API's auth hash is derived from — is set on `.google.com`. Taking
/// only one yields a client that looks signed in and returns nothing.
fn harvest_cookies(win: &WebviewWindow) -> Vec<YtCookie> {
    let mut out: Vec<YtCookie> = Vec::new();
    for url in cookies::COOKIE_URLS {
        let Ok(parsed) = url.parse() else { continue };
        let Ok(found) = win.cookies_for_url(parsed) else {
            continue;
        };
        for c in found {
            let mapped = map_cookie(&c);
            // The same cookie comes back for several of these URLs; keeping
            // duplicates would double it up in the cookies.txt we hand yt-dlp.
            if !out
                .iter()
                .any(|e| e.name == mapped.name && e.domain == mapped.domain)
            {
                out.push(mapped);
            }
        }
    }
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

/* ---- playback ---- */

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
    let resolver: StreamResolver = Arc::new(move |i: usize| {
        let item = items
            .get(i)
            .ok_or_else(|| "queue index out of range".to_string())?;
        let (url, headers) = app
            .state::<YtMusicState>()
            .stream_target(&item.video_id)?;
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
