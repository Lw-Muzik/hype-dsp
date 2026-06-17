//! Error types shared across crates, plus the serializable shape that crosses
//! the Tauri IPC boundary.

use serde::{Deserialize, Serialize};

/// Top-level error for `hm-core` operations (persistence, presets, licensing).
///
/// Each crate defines its own typed error with `thiserror`; at the Tauri
/// boundary they are all converted into a flat [`IpcError`] so the front end
/// receives a stable, serializable shape regardless of which layer failed.
#[derive(Debug, thiserror::Error)]
pub enum HmError {
    /// A requested entity (preset, profile, station) was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The input was structurally invalid (out-of-range band, bad id, …).
    #[error("invalid input: {0}")]
    Invalid(String),

    /// A persistence-layer failure (DB open/query/migration).
    #[error("storage error: {0}")]
    Storage(String),

    /// Serialization or deserialization of a stored/transferred value failed.
    #[error("serialization error: {0}")]
    Serde(String),
}

impl From<serde_json::Error> for HmError {
    fn from(e: serde_json::Error) -> Self {
        HmError::Serde(e.to_string())
    }
}

/// A flat, serializable error returned to the UI from every Tauri command.
///
/// Commands return `Result<T, IpcError>`; the front end's typed IPC wrappers
/// (`src/lib/ipc.ts`) surface `code`/`message` rather than parsing free text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    /// Stable machine-readable code, e.g. `not_found`, `invalid`, `device`.
    pub code: String,
    /// Human-readable detail safe to show in the UI.
    pub message: String,
}

impl IpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<HmError> for IpcError {
    fn from(e: HmError) -> Self {
        let code = match &e {
            HmError::NotFound(_) => "not_found",
            HmError::Invalid(_) => "invalid",
            HmError::Storage(_) => "storage",
            HmError::Serde(_) => "serde",
        };
        IpcError::new(code, e.to_string())
    }
}
