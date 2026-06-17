//! Typed errors for platform integration.

/// Failures from per-app mixer control or platform capabilities.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The feature is not supported on the current platform (e.g. per-process
    /// volume on macOS). The UI treats this as an informational state.
    #[error("not supported on this platform: {0}")]
    Unsupported(String),

    /// The referenced audio session no longer exists.
    #[error("audio session not found: {0}")]
    SessionNotFound(String),

    /// An underlying OS audio API call failed.
    #[error("platform audio error: {0}")]
    Os(String),
}
