//! The licensing seam.
//!
//! HypeMuzik gates a trial/activation flow behind the [`LicenseService`] trait.
//! The shipped implementation (added in Phase 6) is an explicitly-marked local
//! **mock** that persists trial/license state to disk — there is no real DRM,
//! key cryptography, or activation server here, and the app must never imply
//! otherwise. The trait is the seam a real backend would slot into later; the
//! production contract is documented in `docs/architecture.md`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Trial length for the mock, in days.
pub const TRIAL_DAYS: u64 = 14;

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default, Serialize, Deserialize)]
struct MockState {
    first_run_millis: u64,
    key: Option<String>,
}

/// **Local mock** `LicenseService`. Persists trial start + activation key to a
/// JSON file on disk. There is NO real DRM, key cryptography, or activation
/// server — entering any non-empty key flips to `Licensed`. The production
/// contract is documented in `docs/architecture.md`.
pub struct LicenseMock {
    path: PathBuf,
    state: Mutex<MockState>,
}

impl LicenseMock {
    /// Open (or initialize) the mock license state at `path`.
    pub fn open(path: PathBuf) -> Self {
        let mut state: MockState = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if state.first_run_millis == 0 {
            state.first_run_millis = now_millis();
            persist(&path, &state);
        }
        Self {
            path,
            state: Mutex::new(state),
        }
    }
}

fn persist(path: &PathBuf, state: &MockState) -> bool {
    serde_json::to_string(state)
        .ok()
        .and_then(|json| std::fs::write(path, json).ok())
        .is_some()
}

impl LicenseService for LicenseMock {
    fn status(&self) -> LicenseStatus {
        let state = self.state.lock().expect("license state poisoned");
        if state.key.is_some() {
            return LicenseStatus::Licensed;
        }
        let elapsed_days =
            now_millis().saturating_sub(state.first_run_millis) / (1000 * 60 * 60 * 24);
        if elapsed_days >= TRIAL_DAYS {
            LicenseStatus::Expired
        } else {
            LicenseStatus::Trial {
                days_left: (TRIAL_DAYS - elapsed_days) as u32,
            }
        }
    }

    fn activate(&self, key: &str) -> Result<LicenseStatus, LicenseError> {
        if key.trim().is_empty() {
            return Err(LicenseError::InvalidKey);
        }
        let mut state = self.state.lock().expect("license state poisoned");
        state.key = Some(key.trim().to_string());
        if !persist(&self.path, &state) {
            return Err(LicenseError::Storage("could not write license file".into()));
        }
        Ok(LicenseStatus::Licensed)
    }

    fn deactivate(&self) -> Result<(), LicenseError> {
        let mut state = self.state.lock().expect("license state poisoned");
        state.key = None;
        if !persist(&self.path, &state) {
            return Err(LicenseError::Storage("could not write license file".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_is_in_trial() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("hm_license_test_{}.json", now_millis()));
        let mock = LicenseMock::open(path.clone());
        assert!(matches!(
            mock.status(),
            LicenseStatus::Trial { days_left } if u64::from(days_left) == TRIAL_DAYS
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn activation_flips_to_licensed_and_persists() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("hm_license_test2_{}.json", now_millis()));
        {
            let mock = LicenseMock::open(path.clone());
            assert!(mock.activate("ANY-KEY-123").is_ok());
            assert_eq!(mock.status(), LicenseStatus::Licensed);
            assert!(mock.activate("").is_err());
        }
        // Reopen: activation persisted.
        let reopened = LicenseMock::open(path.clone());
        assert_eq!(reopened.status(), LicenseStatus::Licensed);
        reopened.deactivate().unwrap();
        assert!(matches!(reopened.status(), LicenseStatus::Trial { .. }));
        let _ = std::fs::remove_file(&path);
    }
}
