//! `hm-audio` — getting audio in and out.
//!
//! This crate abstracts where audio comes from ([`AudioSource`]) and where it
//! goes ([`AudioSink`]), enumerates devices via `cpal`, and (from Phase 2) hosts
//! the real-time engine that pumps a source through the [`hm_dsp::ProcessChain`]
//! to a sink.
//!
//! Phase 0 establishes the trait surface and a working device-enumeration
//! helper so the audio backend is proven to link against the platform
//! (CoreAudio on macOS) before any streaming code is written.

pub mod decode;
pub mod device;
pub mod engine;
pub mod error;
pub mod sources;
pub mod spectrum;

pub use decode::{decode_file, resample_stereo, DecodedAudio};
pub use device::{list_input_devices, list_output_devices, DeviceInfo};
pub use engine::{AudioEngine, EngineMeters, Renderer};
pub use error::AudioError;
pub use sources::FilePlaybackSource;
pub use spectrum::{SpectrumTap, SPECTRUM_BANDS};

use serde::{Deserialize, Serialize};

/// The PCM format an audio stream runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl StreamFormat {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}

impl Default for StreamFormat {
    /// CD-adjacent stereo default used until a device negotiates its own.
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

/// A producer of interleaved `f32` audio frames.
///
/// Implemented by file playback, loopback capture, and the documented virtual
/// device stub. `read` is pull-based and called from the real-time path, so
/// implementations must avoid blocking.
pub trait AudioSource: Send {
    /// Begin producing audio in `format`.
    fn start(&mut self, format: StreamFormat) -> Result<(), AudioError>;
    /// Fill `out` with up to its capacity of interleaved samples; returns the
    /// number of frames actually written (0 at end-of-stream).
    fn read(&mut self, out: &mut [f32], channels: usize) -> usize;
    /// Stop producing and release resources.
    fn stop(&mut self);

    /// Seek to a frame index. No-op for non-seekable sources (e.g. radio).
    fn seek(&mut self, _frame: usize) {}
    /// Current playback position, in frames.
    fn position(&self) -> usize {
        0
    }
    /// Total length in frames, if known (0 for open-ended streams).
    fn total_frames(&self) -> usize {
        0
    }
}

/// A consumer of interleaved `f32` audio frames (typically the output device).
pub trait AudioSink: Send {
    /// Begin accepting audio in `format`.
    fn start(&mut self, format: StreamFormat) -> Result<(), AudioError>;
    /// Write interleaved samples to the sink.
    fn write(&mut self, buf: &[f32], channels: usize);
    /// Stop accepting and release resources.
    fn stop(&mut self);
}
