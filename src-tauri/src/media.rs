//! OS media controls / "now playing" integration via `souvlaki`.
//!
//! Mirrors the engine's transport into the operating system's media surface —
//! macOS Control Center + media keys, Windows System Media Transport Controls,
//! Linux MPRIS — and forwards the user's hardware/OS transport actions
//! (play/pause/next/previous/seek) back to the UI over the `media:command`
//! event, where the store applies them with full queue/shuffle/repeat context.
//!
//! `souvlaki`'s controls aren't all `Send` across platforms, so they live on a
//! dedicated thread that owns them for the app's lifetime; callers talk to it
//! over a channel.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde::Serialize;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tauri::{AppHandle, Emitter};

/// A transport action the OS asked us to perform, forwarded to the UI.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaCommand {
    /// One of: play, pause, toggle, next, prev, stop, seek, seekForward, seekBackward.
    action: &'static str,
    /// Absolute target (for `seek`) or delta in seconds (for `seekBy`), if any.
    secs: Option<f64>,
}

/// Messages pushed to the media-controls thread.
enum Update {
    Metadata {
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        cover: Option<String>,
        duration_secs: Option<f64>,
    },
    Playback {
        playing: bool,
        paused: bool,
        position_secs: f64,
    },
}

/// Cloneable handle to the media-controls thread. The thread runs until every
/// handle is dropped.
#[derive(Clone)]
pub struct MediaSession {
    tx: Sender<Update>,
}

impl MediaSession {
    /// Spawn the media-controls thread. `hwnd` is only used on Windows.
    pub fn spawn(app: AppHandle, hwnd: Option<isize>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hm-media-controls".into())
            .spawn(move || run(app, hwnd, rx))
            .ok();
        Self { tx }
    }

    /// Update the now-playing card shown by the OS.
    pub fn set_metadata(
        &self,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        cover: Option<String>,
        duration_secs: Option<f64>,
    ) {
        let _ = self.tx.send(Update::Metadata {
            title,
            artist,
            album,
            cover,
            duration_secs,
        });
    }

    /// Update the OS playback status + elapsed position.
    pub fn set_playback(&self, playing: bool, paused: bool, position_secs: f64) {
        let _ = self.tx.send(Update::Playback {
            playing,
            paused,
            position_secs,
        });
    }
}

/// Resolve the main window's `HWND` (Windows-only; `None` elsewhere).
#[cfg(target_os = "windows")]
fn main_hwnd(app: &AppHandle) -> Option<isize> {
    use tauri::Manager;
    app.get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn main_hwnd(_app: &AppHandle) -> Option<isize> {
    None
}

/// Convenience used by `lib.rs` setup: resolve the hwnd and spawn the session.
pub fn start(app: AppHandle) -> MediaSession {
    let hwnd = main_hwnd(&app);
    MediaSession::spawn(app, hwnd)
}

fn run(app: AppHandle, hwnd: Option<isize>, rx: Receiver<Update>) {
    let config = PlatformConfig {
        display_name: "HypeMuzik",
        dbus_name: "hypemuzik",
        hwnd: hwnd.map(|h| h as *mut std::ffi::c_void),
    };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(_) => return, // media controls unavailable on this platform/session
    };

    let cmd_app = app.clone();
    let _ = controls.attach(move |event: MediaControlEvent| {
        let (action, secs) = match event {
            MediaControlEvent::Play => ("play", None),
            MediaControlEvent::Pause => ("pause", None),
            MediaControlEvent::Toggle => ("toggle", None),
            MediaControlEvent::Next => ("next", None),
            MediaControlEvent::Previous => ("prev", None),
            MediaControlEvent::Stop => ("stop", None),
            MediaControlEvent::SetPosition(MediaPosition(d)) => ("seek", Some(d.as_secs_f64())),
            MediaControlEvent::SeekBy(SeekDirection::Forward, d) => {
                ("seekForward", Some(d.as_secs_f64()))
            }
            MediaControlEvent::SeekBy(SeekDirection::Backward, d) => {
                ("seekBackward", Some(d.as_secs_f64()))
            }
            MediaControlEvent::Seek(SeekDirection::Forward) => ("seekForward", None),
            MediaControlEvent::Seek(SeekDirection::Backward) => ("seekBackward", None),
            _ => return,
        };
        let _ = cmd_app.emit("media:command", MediaCommand { action, secs });
    });

    while let Ok(update) = rx.recv() {
        match update {
            Update::Metadata {
                title,
                artist,
                album,
                cover,
                duration_secs,
            } => {
                let _ = controls.set_metadata(MediaMetadata {
                    title: title.as_deref(),
                    artist: artist.as_deref(),
                    album: album.as_deref(),
                    cover_url: cover.as_deref(),
                    duration: duration_secs.map(Duration::from_secs_f64),
                });
            }
            Update::Playback {
                playing,
                paused,
                position_secs,
            } => {
                let progress = Some(MediaPosition(Duration::from_secs_f64(position_secs.max(0.0))));
                let state = if !playing {
                    MediaPlayback::Stopped
                } else if paused {
                    MediaPlayback::Paused { progress }
                } else {
                    MediaPlayback::Playing { progress }
                };
                let _ = controls.set_playback(state);
            }
        }
    }
}
