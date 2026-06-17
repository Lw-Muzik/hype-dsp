//! `hm-media` — local playback and internet radio.
//!
//! Both subsystems decode audio and feed it through the same
//! [`hm_dsp::ProcessChain`](../hm_dsp/index.html) as the system enhancer, so a
//! played file or a live stream is heard with the active EQ/effects applied.
//!
//! Phase 0 establishes the transport vocabulary and error type. The player
//! (library/playlist, symphonia decode) and radio (radio-browser directory,
//! reqwest streaming, favorites) are implemented in Phase 5.

use serde::{Deserialize, Serialize};

pub mod error;
pub mod radio;
pub use error::MediaError;

/// Playback transport state shared with the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransportState {
    #[default]
    Stopped,
    Playing,
    Paused,
    Buffering,
}

/// Now-playing position, emitted to the UI as playback progresses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportProgress {
    /// Elapsed time in seconds.
    pub position_secs: f64,
    /// Total duration in seconds, if known (streams may be open-ended).
    pub duration_secs: Option<f64>,
}
