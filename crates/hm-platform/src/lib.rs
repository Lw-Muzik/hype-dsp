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
#[cfg(target_os = "macos")]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(UnsupportedSessionController::new(
        "Per-application volume isn't available on macOS — it requires process-tap \
         interception (a system extension), which HypeMuzik does not install.",
    ))
}

/// The per-app mixer controller for the current platform.
///
/// Windows supports this natively; see [`windows_notes`] for the implementation
/// plan. It is scaffolded as unsupported here because this build was produced
/// on macOS and the COM code could not be compile-verified.
#[cfg(target_os = "windows")]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(UnsupportedSessionController::new(
        "Windows per-app volume implementation pending (IAudioSessionManager2).",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn default_controller() -> Box<dyn SessionController> {
    Box::new(UnsupportedSessionController::new(
        "Per-application volume isn't supported on this platform.",
    ))
}

/// Production plan for the **Windows** per-app mixer (Module 3).
///
/// Windows exposes per-process volume natively; a real `SessionController`
/// there would, via the `windows` crate:
///
/// 1. `CoInitializeEx`, then `CoCreateInstance::<IMMDeviceEnumerator>(MMDeviceEnumerator)`.
/// 2. `GetDefaultAudioEndpoint(eRender, eMultimedia)` → `IMMDevice`.
/// 3. `device.Activate::<IAudioSessionManager2>(CLSCTX_ALL)`.
/// 4. `GetSessionEnumerator()` → iterate `IAudioSessionControl`, cast each to
///    `IAudioSessionControl2` for `GetProcessId` / `GetSessionIdentifier`, and
///    resolve the process name via `OpenProcess` + `QueryFullProcessImageName`.
/// 5. Cast each control to `ISimpleAudioVolume` for
///    `SetMasterVolume` / `SetMute` (per-app volume) and `GetMasterVolume`.
///
/// All calls are `unsafe` COM and must be serialized (apartment-threaded), so
/// the controller is held behind a `Mutex`. This was not compiled here (macOS
/// host); it needs a Windows build to verify.
pub mod windows_notes {}
