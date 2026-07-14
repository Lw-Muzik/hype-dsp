//! `hm-video` — native TV/video playback by driving an **mpv** child process.
//!
//! TV channels are video (almost always HLS), so they can't go through the
//! audio engine. Instead of linking a media library into the app, we spawn the
//! **mpv** player as a separate process that owns its own native window and its
//! built-in on-screen controller (play/pause, seek, volume, fullscreen). mpv is
//! ffmpeg-backed, so it plays every container/codec/protocol that exists — the
//! "like VLC, all formats" requirement — and does its own native HTTP with the
//! per-stream `User-Agent`/`Referer` many IPTV streams require, so the webview's
//! CSP is never involved and no local proxy is needed.
//!
//! Running mpv as a *separate, unmodified process* (aggregation, not linking)
//! also keeps its GPL cleanly separated from the host application, exactly like
//! launching VLC would.
//!
//! Only one channel plays at a time: [`VideoPlayer::play`] replaces any running
//! mpv with a fresh one for the new stream. This module is intentionally
//! Tauri-agnostic — the command layer resolves the bundled binary path and maps
//! the app's `TvChannel` onto a [`PlayRequest`].

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("the TV player (mpv) could not be started: {0}")]
    Spawn(String),
}

/// A request to play one channel in the native window.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    /// The stream URL (typically an `.m3u8` HLS playlist).
    pub url: String,
    /// Human title shown in mpv's window/OSD (the channel name).
    pub title: String,
    /// `User-Agent` header the stream requires, if any.
    pub user_agent: Option<String>,
    /// `Referer` header the stream requires, if any.
    pub referrer: Option<String>,
}

/// Build mpv's argument vector for a play request. Pure and deterministic so it
/// can be unit-tested without spawning anything.
///
/// Notes:
/// - `--force-window=immediate` shows the window at once (before the stream has
///   resolved), so a slow live stream doesn't look like nothing happened.
/// - `--osc=yes` gives the built-in VLC-style controller.
/// - `--` terminates option parsing so a stream URL is never mistaken for a flag.
fn build_args(req: &PlayRequest) -> Vec<String> {
    let mut args = vec![
        "--force-window=immediate".to_string(),
        "--osc=yes".to_string(),
        "--keep-open=no".to_string(),
        "--idle=no".to_string(),
        // Live-stream resilience: a modest demuxer cache + a bounded network
        // timeout so a stalled server surfaces as an error instead of hanging.
        "--cache=yes".to_string(),
        "--network-timeout=30".to_string(),
        format!("--force-media-title={}", req.title),
        format!("--title=HypeMuzik TV — {}", req.title),
    ];
    if let Some(ua) = req.user_agent.as_deref().filter(|s| !s.is_empty()) {
        args.push(format!("--user-agent={ua}"));
    }
    if let Some(referrer) = req.referrer.as_deref().filter(|s| !s.is_empty()) {
        args.push(format!("--referrer={referrer}"));
    }
    args.push("--".to_string());
    args.push(req.url.clone());
    args
}

/// Drives a single mpv process. Cheap to construct; the process only exists
/// while something is playing.
pub struct VideoPlayer {
    /// Path to the mpv binary — the bundled resource in a packaged app, or plain
    /// `mpv` (resolved on `PATH`) in development.
    binary: PathBuf,
    child: Mutex<Option<Child>>,
}

impl VideoPlayer {
    /// Create a player that launches `binary`. Pass the bundled mpv path, or
    /// `PathBuf::from("mpv")` to use whatever is on `PATH`.
    pub fn new(binary: PathBuf) -> Self {
        Self { binary, child: Mutex::new(None) }
    }

    /// Play (or switch to) a channel. Any currently-playing stream is stopped
    /// first, so only one native window is ever open.
    pub fn play(&self, req: PlayRequest) -> Result<(), VideoError> {
        let mut guard = self.child.lock().expect("video player poisoned");
        kill(&mut guard);
        let child = Command::new(&self.binary)
            .args(build_args(&req))
            .spawn()
            .map_err(|e| VideoError::Spawn(e.to_string()))?;
        *guard = Some(child);
        Ok(())
    }

    /// Stop playback and close the window, if open.
    pub fn stop(&self) {
        let mut guard = self.child.lock().expect("video player poisoned");
        kill(&mut guard);
    }

    /// Whether a channel is currently playing. Reaps the child if the window was
    /// closed by the user (or the process died), so callers can poll this to
    /// clear a "now watching" indicator.
    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().expect("video player poisoned");
        match guard.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None; // exited — reap it
                    false
                }
                Ok(None) => true, // still alive
                Err(_) => {
                    *guard = None;
                    false
                }
            },
            None => false,
        }
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            kill(&mut guard);
        }
    }
}

/// Kill and reap the child, if any. Best-effort: a stream that already exited is
/// simply cleared.
fn kill(guard: &mut Option<Child>) {
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> PlayRequest {
        PlayRequest {
            url: "https://example.com/live/stream.m3u8".to_string(),
            title: "Test Channel".to_string(),
            user_agent: None,
            referrer: None,
        }
    }

    #[test]
    fn args_show_a_window_and_the_url_last_after_a_terminator() {
        let args = build_args(&req());
        assert!(args.iter().any(|a| a.starts_with("--force-window")));
        assert!(args.iter().any(|a| a == "--osc=yes"));
        // The URL is the final arg, immediately preceded by `--`.
        assert_eq!(args.last().unwrap(), "https://example.com/live/stream.m3u8");
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(dashdash, args.len() - 2);
    }

    #[test]
    fn headers_are_passed_only_when_present_and_non_empty() {
        let mut r = req();
        r.user_agent = Some("Mozilla/5.0".to_string());
        r.referrer = Some(String::new()); // empty → skipped
        let args = build_args(&r);
        assert!(args.iter().any(|a| a == "--user-agent=Mozilla/5.0"));
        assert!(!args.iter().any(|a| a.starts_with("--referrer")));
    }

    #[test]
    fn title_becomes_the_media_title() {
        let args = build_args(&req());
        assert!(args.iter().any(|a| a == "--force-media-title=Test Channel"));
    }

    #[test]
    fn a_missing_binary_surfaces_as_a_spawn_error() {
        let player = VideoPlayer::new(PathBuf::from("definitely-not-a-real-mpv-binary-xyz"));
        assert!(matches!(player.play(req()), Err(VideoError::Spawn(_))));
        assert!(!player.is_running());
    }
}
