//! Typed errors for the media subsystems.

/// Failures from local playback or radio streaming.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// The file or stream could not be opened.
    #[error("could not open media: {0}")]
    Open(String),

    /// The container or codec is unsupported by the decoder.
    #[error("unsupported media format: {0}")]
    UnsupportedFormat(String),

    /// A decode error mid-stream.
    #[error("decode error: {0}")]
    Decode(String),

    /// A network error while fetching a stream or the station directory.
    #[error("network error: {0}")]
    Network(String),
}
