//! `hm-platform` — OS-specific audio integration.
//!
//! Two responsibilities, both behind traits so the rest of the app is
//! platform-agnostic:
//!
//! 1. **Per-application mixer** ([`SessionController`]) — list and attenuate
//!    individual app audio streams. Real on Windows (audio session APIs); a
//!    documented unsupported stub on macOS, which the UI degrades to cleanly.
//! 2. **Virtual capture device** — the seam for true system-wide capture. No
//!    signed driver can be produced in-session, so this ships as a documented
//!    `Unavailable` stub (Phase 6) with the production requirements written up
//!    in `docs/audio-driver.md`.
//!
//! Phase 0 establishes the [`SessionController`] trait and error type.

use hm_core::AppSession;

pub mod error;
pub use error::PlatformError;

/// Lists and controls per-application audio sessions.
///
/// The concrete implementation is selected by target OS at build time. Where a
/// platform cannot support per-process control, its implementation reports an
/// empty list and the UI shows a clear "unavailable on this platform" state
/// rather than fake controls.
pub trait SessionController: Send {
    /// Snapshot of the current per-app sessions.
    fn list_sessions(&self) -> Vec<AppSession>;
    /// Set a session's linear volume (0.0–1.0).
    fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError>;
    /// Mute or unmute a session.
    fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError>;
}
