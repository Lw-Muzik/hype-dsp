//! The licensing seam.
//!
//! HypeMuzik gates a trial/activation flow behind the [`LicenseService`] trait.
//! The shipped implementation (added in Phase 6) is an explicitly-marked local
//! **mock** that persists trial/license state to disk — there is no real DRM,
//! key cryptography, or activation server here, and the app must never imply
//! otherwise. The trait is the seam a real backend would slot into later; the
//! production contract is documented in `docs/architecture.md`.

use serde::{Deserialize, Serialize};

/// The user's current entitlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LicenseStatus {
    /// Trial active, with whole days remaining.
    Trial { days_left: u32 },
    /// A key has been activated (mock: any non-empty key).
    Licensed,
    /// Trial elapsed and no key activated.
    Expired,
}

/// Failure modes for activation/deactivation.
#[derive(Debug, thiserror::Error)]
pub enum LicenseError {
    #[error("invalid license key")]
    InvalidKey,
    #[error("license storage error: {0}")]
    Storage(String),
}

/// Abstracts trial/activation so the UI never depends on how entitlement is
/// resolved. Implemented by a local mock today; replaceable by a networked
/// service without touching the front end.
pub trait LicenseService: Send + Sync {
    /// Current entitlement, resolving trial expiry as needed.
    fn status(&self) -> LicenseStatus;
    /// Activate with a key. The mock accepts any non-empty key.
    fn activate(&self, key: &str) -> Result<LicenseStatus, LicenseError>;
    /// Clear activation, returning to trial/expired.
    fn deactivate(&self) -> Result<(), LicenseError>;
}
