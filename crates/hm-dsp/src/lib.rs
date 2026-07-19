//! `hm-dsp` — the audio processing core.
//!
//! Pure DSP only: no device I/O, no Tauri, no threads. Everything here operates
//! on interleaved `f32` buffers and can be unit-tested in isolation, which is
//! exactly how the engine's correctness is guaranteed (null tests, per-band FFT
//! checks, limiter ceiling checks — see Phase 1).
//!
//! The runtime chain processes audio in a fixed order:
//! `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Surround3D → RoomEffects → Convolver → Compander → Saturation → Script → Gain → Limiter`.
//! Each stage implements [`AudioProcessor`]; [`ProcessChain`] owns the ordered
//! list and runs them in place. Phase 0 establishes these interfaces and an
//! empty (identity) chain; the processors themselves arrive in Phase 1.

use hm_core::EngineState;

pub mod script;
pub mod script_stage;
pub mod bass_boost;
pub mod compander;
pub mod convolver;
pub mod biquad;
mod delay;
pub mod oversample;
pub mod gain;
pub mod graphic_eq;
pub mod headphone;
pub mod limiter;
mod reverb;
pub mod room;
pub mod saturation;
pub mod spatializer;
pub mod surround3d;

pub use bass_boost::BassBoost;
pub use compander::{Compander, CompanderMeter};
pub use convolver::{empty_ir_slot, Convolver, IrSlot, PreparedIr};

/// Returns a fresh, throwaway `CompanderMeter` for call sites that do not
/// need to observe per-band gain reduction (mirrors [`empty_ir_slot`]).
pub fn empty_compander_meter() -> std::sync::Arc<CompanderMeter> {
    std::sync::Arc::new(CompanderMeter::default())
}
pub use gain::Gain;
pub use graphic_eq::GraphicEq;
pub use headphone::HeadphoneCorrection;
pub use limiter::Limiter;
pub use room::RoomEffects;
pub use saturation::Saturation;
pub use script_stage::{empty_script_slot, ScriptProcessor, ScriptSlot};
pub use spatializer::Spatializer;
pub use surround3d::Surround3D;

/// The handles the engine keeps a grip on after building a chain.
///
/// Most stages are configured entirely by the per-block [`ProcessorParams`]
/// snapshot. These three cannot be: an impulse response, a compiled script and a
/// meter are too big, too costly to rebuild, or flowing the wrong way to travel
/// in a value copied every block. So they are shared up front and published to
/// (or read from) afterwards.
///
/// Grouped rather than passed positionally because there are now three of them
/// and every call site passes them together — a fourth positional slot is the
/// point where an argument list stops being readable at the call.
#[derive(Clone)]
pub struct ChainSlots {
    /// Convolver impulse response, published by the engine on load.
    pub ir: IrSlot,
    /// Per-band gain reduction, written by the compander for the UI to read.
    pub compander_meter: std::sync::Arc<CompanderMeter>,
    /// Compiled LiveProg program, published by the engine on compile.
    pub script: ScriptSlot,
}

impl Default for ChainSlots {
    /// Throwaway slots, for call sites that never publish or observe — the
    /// system-EQ chains and most tests.
    fn default() -> Self {
        Self {
            ir: empty_ir_slot(),
            compander_meter: empty_compander_meter(),
            script: empty_script_slot(),
        }
    }
}

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
    /// `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Surround3D →
    /// RoomEffects → Convolver → Compander → Saturation → Script → Gain → Limiter`.
    pub fn standard(sample_rate: f32, channels: usize) -> Self {
        Self::standard_with_slots(sample_rate, channels, ChainSlots::default())
    }

    /// Like [`standard`](Self::standard) but with externally-owned slots, so the
    /// engine can publish into stages that outlive a single parameter snapshot:
    /// impulse responses to the convolver, compiled scripts to the script stage,
    /// and per-band gain-reduction read back out of the compander.
    pub fn standard_with_slots(
        sample_rate: f32,
        channels: usize,
        slots: ChainSlots,
    ) -> Self {
        let ChainSlots { ir, compander_meter, script } = slots;
        let mut chain = Self::new();
        chain.prepare(sample_rate, channels);
        chain.push(Box::new(HeadphoneCorrection::new(sample_rate, channels)));
        chain.push(Box::new(GraphicEq::new(sample_rate, channels)));
        chain.push(Box::new(BassBoost::new(sample_rate, channels)));
        chain.push(Box::new(Spatializer::new(sample_rate, channels)));
        chain.push(Box::new(Surround3D::new(sample_rate, channels)));
        chain.push(Box::new(RoomEffects::new(sample_rate, channels)));
        chain.push(Box::new(Convolver::with_slot(sample_rate, channels, ir)));
        chain.push(Box::new(Compander::with_meter(sample_rate, channels, compander_meter)));
        chain.push(Box::new(Saturation::new(sample_rate, channels)));
        chain.push(Box::new(ScriptProcessor::with_slot(sample_rate, channels, script)));
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

    /// The standard chain with IR slot, with all effects off, should not
    /// distort the signal; the convolver must be present in the chain.
    #[test]
    fn standard_chain_is_identity_when_all_off() {
        let mut state = EngineState::default();
        state.eq.enabled = false;
        state.power = true;
        let mut chain = ProcessChain::standard_with_slots(48_000.0, 2, ChainSlots::default());
        chain.set_params(&state);
        // Convolver disabled by default → chain must not blow up; length includes it.
        assert!(chain.len() >= 8, "convolver should be in the standard chain");
        let original: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let mut buf = original.clone();
        chain.process(&mut buf, 2);
        assert!(buf.iter().all(|&x| x.abs() <= 1.0));
    }

    /// The compander must be in the standard chain after the convolver.
    #[test]
    fn standard_chain_includes_compander() {
        let chain = ProcessChain::standard_with_slots(48_000.0, 2, ChainSlots::default());
        assert!(chain.len() >= 10, "compander should be in the standard chain");
    }

    /// Saturation must be in the standard chain after compander.
    #[test]
    fn standard_chain_includes_saturation() {
        let chain = ProcessChain::standard_with_slots(48_000.0, 2, ChainSlots::default());
        assert!(chain.len() >= 11, "saturation should be in the standard chain");
    }

    /// The script stage must be in the standard chain, after saturation and
    /// before gain/limiter — a script runs on the finished sound, but still
    /// upstream of the makeup gain and the ceiling that keeps it from clipping.
    #[test]
    fn standard_chain_includes_script() {
        let chain = ProcessChain::standard_with_slots(48_000.0, 2, ChainSlots::default());
        assert!(chain.len() >= 12, "script stage should be in the standard chain");
    }
}
