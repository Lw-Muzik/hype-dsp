//! `hm-platform` — OS-specific audio integration.
//!
//! Two responsibilities, both behind traits so the rest of the app is
//! platform-agnostic:
//!
//! 1. **Per-application mixer** ([`SessionController`]) — list and attenuate
//!    individual app audio streams. Windows can do this natively (audio session
//!    APIs); macOS cannot without deeper process-tap interception, so it
//!    reports "unsupported" and the UI degrades to a clear notice.
//! 2. **Virtual capture device** — the seam for true system-wide capture. No
//!    signed driver can be produced in-session, so this ships as a documented
//!    `Unavailable` stub with the production requirements in
//!    `docs/audio-driver.md`.

use hm_core::AppSession;

pub mod error;
pub use error::PlatformError;

// Shared helpers (base64 / data URIs) for the platforms that build icons.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod util;

#[cfg(target_os = "macos")]
mod macos;

// Mounted as `win` (not `windows`) so it can't shadow the `windows` crate.
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod win;

/// Lists and controls per-application audio sessions.
pub trait SessionController: Send {
    /// Whether per-app control is available on this platform/build.
    fn supported(&self) -> bool {
        false
    }
    /// A human-readable reason when unsupported (for the UI).
    fn unavailable_reason(&self) -> Option<String> {
        None
    }
    /// Snapshot of the current per-app sessions (empty when unsupported).
    fn list_sessions(&self) -> Vec<AppSession>;
    /// Set a session's linear volume (0.0–1.0).
    fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError>;
    /// Mute or unmute a session.
    fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError>;
}

/// A controller that reports the feature as unsupported with a reason. Used on
/// macOS today, and as the Windows placeholder until the native implementation
/// is compiled and verified there (see [`windows_notes`]).
pub struct UnsupportedSessionController {
    reason: String,
}

impl UnsupportedSessionController {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl SessionController for UnsupportedSessionController {
    fn supported(&self) -> bool {
        false
    }
    fn unavailable_reason(&self) -> Option<String> {
        Some(self.reason.clone())
    }
    fn list_sessions(&self) -> Vec<AppSession> {
        Vec::new()
    }
    fn set_volume(&self, _id: &str, _gain: f32) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(self.reason.clone()))
    }
    fn set_muted(&self, _id: &str, _muted: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(self.reason.clone()))
    }
}

/// The per-app mixer controller for the current platform.
///
/// macOS lists audio apps via Core Audio process enumeration (Phase 1); the
/// tap-based attenuation engine arrives in Phases 2–3.
#[cfg(target_os = "macos")]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(macos::MacosSessionController::new())
}

/// The per-app mixer controller for the current platform.
///
/// Windows uses WASAPI audio sessions (`IAudioSessionManager2` /
/// `ISimpleAudioVolume`) natively — see [`win`]. Built/verified by CI on
/// Windows, not on the macOS dev host.
#[cfg(target_os = "windows")]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(win::WindowsSessionController::new())
}

/// Linux (PulseAudio/PipeWire sink-input volume) is a future phase; until then
/// it degrades to a clear notice.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(UnsupportedSessionController::new(
        "Per-application volume isn't supported on this platform yet.",
    ))
}
