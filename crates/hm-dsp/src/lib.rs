//! `hm-dsp` — the audio processing core.
//!
//! Pure DSP only: no device I/O, no Tauri, no threads. Everything here operates
//! on interleaved `f32` buffers and can be unit-tested in isolation, which is
//! exactly how the engine's correctness is guaranteed (null tests, per-band FFT
//! checks, limiter ceiling checks — see Phase 1).
//!
//! The runtime chain processes audio in a fixed order:
//! `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Surround3D → RoomEffects → Gain → Limiter`.
//! Each stage implements [`AudioProcessor`]; [`ProcessChain`] owns the ordered
//! list and runs them in place. Phase 0 establishes these interfaces and an
//! empty (identity) chain; the processors themselves arrive in Phase 1.

use hm_core::EngineState;

pub mod bass_boost;
pub mod biquad;
mod delay;
pub mod gain;
pub mod graphic_eq;
pub mod headphone;
pub mod limiter;
mod reverb;
pub mod room;
pub mod spatializer;
pub mod surround3d;

pub use bass_boost::BassBoost;
pub use gain::Gain;
pub use graphic_eq::GraphicEq;
pub use headphone::HeadphoneCorrection;
pub use limiter::Limiter;
pub use room::RoomEffects;
pub use spatializer::Spatializer;
pub use surround3d::Surround3D;

/// Immutable per-block parameter snapshot handed to processors.
///
/// The audio thread reads one of these at the top of every block. It is derived
/// from [`hm_core::EngineState`]; for now it is that state directly, and will
/// grow precomputed, sample-rate-aware coefficients as processors are added.
pub type ProcessorParams = EngineState;

/// A single in-place audio processing stage.
///
/// Implementors must not allocate, lock, or perform I/O inside [`process`]:
/// it runs on the real-time audio callback thread.
///
/// [`process`]: AudioProcessor::process
pub trait AudioProcessor: Send {
    /// Called off the audio thread when the stream format is known or changes.
    /// Processors size their internal state (filter histories, delay lines)
    /// here so [`process`](AudioProcessor::process) never allocates.
    fn prepare(&mut self, sample_rate: f32, channels: usize);

    /// Process `buffer` in place. Samples are interleaved by `channels`.
    fn process(&mut self, buffer: &mut [f32], channels: usize);

    /// Apply a new parameter snapshot. Cheap and allocation-free.
    fn set_params(&mut self, params: &ProcessorParams);
}

/// An ordered collection of [`AudioProcessor`]s applied in sequence.
///
/// An empty chain is the identity transform — audio passes through bit-exact.
/// This is the honest Phase 0 state: nothing claims to enhance audio until the
/// real processors are added in Phase 1.
#[derive(Default)]
pub struct ProcessChain {
    processors: Vec<Box<dyn AudioProcessor>>,
    sample_rate: f32,
    channels: usize,
}

impl ProcessChain {
    /// Create an empty chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the standard enhancement chain for the given format, in the
    /// canonical fixed order:
    /// `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Gain →
    /// Limiter`.
    pub fn standard(sample_rate: f32, channels: usize) -> Self {
        let mut chain = Self::new();
        chain.prepare(sample_rate, channels);
        chain.push(Box::new(HeadphoneCorrection::new(sample_rate, channels)));
        chain.push(Box::new(GraphicEq::new(sample_rate, channels)));
        chain.push(Box::new(BassBoost::new(sample_rate, channels)));
        chain.push(Box::new(Spatializer::new(sample_rate, channels)));
        chain.push(Box::new(Surround3D::new(sample_rate, channels)));
        chain.push(Box::new(RoomEffects::new(sample_rate, channels)));
        chain.push(Box::new(Gain::new()));
        chain.push(Box::new(Limiter::new(sample_rate, channels)));
        chain
    }

    /// Append a processor to the end of the chain, preparing it if the chain
    /// has already been prepared with a known format.
    pub fn push(&mut self, mut processor: Box<dyn AudioProcessor>) {
        if self.sample_rate > 0.0 {
            processor.prepare(self.sample_rate, self.channels);
        }
        self.processors.push(processor);
    }

    /// Number of stages in the chain.
    pub fn len(&self) -> usize {
        self.processors.len()
    }

    /// Whether the chain has no stages (i.e. is the identity transform).
    pub fn is_empty(&self) -> bool {
        self.processors.is_empty()
    }

    /// Prepare every stage for a (possibly new) stream format.
    pub fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        for p in &mut self.processors {
            p.prepare(sample_rate, channels);
        }
    }

    /// Push a fresh parameter snapshot to every stage.
    pub fn set_params(&mut self, params: &ProcessorParams) {
        for p in &mut self.processors {
            p.set_params(params);
        }
    }

    /// Run every stage over `buffer` in place, in chain order.
    ///
    /// Real-time safe: no allocation, no locking, no I/O.
    pub fn process(&mut self, buffer: &mut [f32], channels: usize) {
        for p in &mut self.processors {
            p.process(buffer, channels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty chain must leave the buffer bit-for-bit unchanged (identity).
    /// This is the foundation the Phase 1 null tests build on.
    #[test]
    fn empty_chain_is_identity() {
        let mut chain = ProcessChain::new();
        chain.prepare(48_000.0, 2);
        assert!(chain.is_empty());

        let original: Vec<f32> = (0..512).map(|i| (i as f32 * 0.001).sin()).collect();
        let mut buffer = original.clone();
        chain.process(&mut buffer, 2);

        assert_eq!(buffer, original, "empty chain must not alter the signal");
    }

    /// The chain reports its length as stages are added.
    #[test]
    fn chain_tracks_length() {
        let chain = ProcessChain::new();
        assert_eq!(chain.len(), 0);
        assert!(chain.is_empty());
    }
}
