//! Typed errors for the audio layer.

/// Failures from device enumeration, stream setup, or capture/playback.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// No device matched the request (e.g. named output not present).
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),

    /// The host backend could not enumerate or open devices.
    #[error("audio host error: {0}")]
    Host(String),

    /// The requested or negotiated stream format is unsupported.
    #[error("unsupported stream format: {0}")]
    UnsupportedFormat(String),

    /// Building or starting the stream failed.
    #[error("stream error: {0}")]
    Stream(String),

    /// Opening or reading the media file failed.
    #[error("could not open file: {0}")]
    Io(String),

    /// Decoding the audio data failed.
    #[error("decode error: {0}")]
    Decode(String),

    /// The capture surface exists but is not available (e.g. the virtual
    /// device driver is not installed). Surfaced to the UI as a clean state,
    /// never as a crash.
    #[error("audio source unavailable: {0}")]
    Unavailable(String),
}

/// Flatten an [`AudioError`] into the IPC error shape the UI receives. Defined
/// here (where `AudioError` is local) so Tauri command handlers can use `?`.
impl From<AudioError> for hm_core::IpcError {
    fn from(e: AudioError) -> Self {
        let code = match &e {
            AudioError::DeviceNotFound(_) => "device_not_found",
            AudioError::Host(_) => "audio_host",
            AudioError::UnsupportedFormat(_) => "unsupported_format",
            AudioError::Stream(_) => "stream",
            AudioError::Io(_) => "io",
            AudioError::Decode(_) => "decode",
            AudioError::Unavailable(_) => "unavailable",
        };
        hm_core::IpcError::new(code, e.to_string())
    }
}
