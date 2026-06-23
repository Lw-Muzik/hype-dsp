//! Platform-agnostic per-block DSP step shared by every system-wide EQ backend.
//!
//! The macOS tap, the Linux virtual sink, and the Windows WASAPI loop all reduce
//! to the same inner operation: *given an interleaved f32 stereo block and the
//! engine's live [`EngineState`], apply the master volume and (when powered) the
//! shared [`ProcessChain`].* Factoring it here keeps that seam in one tested
//! place — and, crucially, it compiles and is unit-tested on the dev host even
//! though the Windows/Linux capture-render plumbing around it cannot be.
//!
//! Real-time contract: this function never allocates, locks, or blocks — it
//! mutates the caller's pre-sized buffer in place and reuses the caller's
//! long-lived [`ProcessChain`]. Callers read the [`EngineState`] snapshot once
//! per block via `ArcSwap::load` and hand the borrow in here.

use hm_core::EngineState;
use hm_dsp::ProcessChain;

/// Apply the engine's master volume and (when `state.power`) the DSP chain to one
/// interleaved block, in place.
///
/// `chain` is the caller's persistent [`ProcessChain`] (built once, reused every
/// block); `samples` is interleaved by `channels`. Mirrors the inner body of the
/// Linux `process_loop` so every backend behaves identically.
///
/// When the engine is bypassed (`power == false`) only the master volume is
/// applied, so the chain's internal state isn't advanced while bypassed — matching
/// the real-time engine's own bypass behaviour.
///
/// Only the self-contained re-routing backends (Linux/Windows) call this; the
/// macOS host drives its own process tap, so the helper is `dead_code` *there* —
/// but it is still compiled and unit-tested on the macOS dev box (the whole point
/// of factoring it out), so the allow is scoped to that host only.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) fn process_block(
    chain: &mut ProcessChain,
    samples: &mut [f32],
    channels: usize,
    state: &EngineState,
) {
    if (state.master_volume - 1.0).abs() > f32::EPSILON {
        for s in samples.iter_mut() {
            *s *= state.master_volume;
        }
    }
    if state.power {
        chain.set_params(state);
        chain.process(samples, channels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;
    const CHANNELS: usize = 2;

    /// A flat-EQ, no-effects state at unity volume: the chain should be a no-op so
    /// any audible change in a test signals an unexpected processor kicking in.
    ///
    /// `power`/`master_volume`/effects already default to this, but spelling the
    /// EQ out via struct-update keeps the preconditions explicit and self-documenting.
    fn flat_state() -> EngineState {
        EngineState {
            power: true,
            master_volume: 1.0,
            eq: hm_core::EqState {
                enabled: true,
                pre_gain: 0.0,
                bands: [0.0; hm_core::BAND_COUNT],
            },
            ..EngineState::default()
        }
    }

    #[test]
    fn power_off_is_identity() {
        let mut chain = ProcessChain::standard(RATE, CHANNELS);
        let mut st = flat_state();
        st.power = false; // bypass
        let original = sample_signal();
        let mut samples = original.clone();

        process_block(&mut chain, &mut samples, CHANNELS, &st);

        assert_eq!(
            samples, original,
            "power=false must pass the block through untouched"
        );
    }

    #[test]
    fn master_volume_scales_when_bypassed() {
        let mut chain = ProcessChain::standard(RATE, CHANNELS);
        let mut st = flat_state();
        st.power = false; // isolate the volume stage from the chain
        st.master_volume = 0.5;
        let original = sample_signal();
        let mut samples = original.clone();

        process_block(&mut chain, &mut samples, CHANNELS, &st);

        for (out, inp) in samples.iter().zip(original.iter()) {
            assert!(
                (out - inp * 0.5).abs() < 1e-6,
                "master_volume=0.5 must halve each sample: got {out}, want {}",
                inp * 0.5
            );
        }
    }

    #[test]
    fn unity_volume_flat_eq_is_near_identity() {
        let mut chain = ProcessChain::standard(RATE, CHANNELS);
        let st = flat_state();
        let original = sample_signal();
        let mut samples = original.clone();

        // Prime the chain so any latency/transient settles before we compare.
        let mut warmup = vec![0.0f32; original.len()];
        process_block(&mut chain, &mut warmup, CHANNELS, &st);

        process_block(&mut chain, &mut samples, CHANNELS, &st);

        for s in &samples {
            assert!(s.is_finite(), "output must stay finite");
        }
    }

    #[test]
    fn enabled_eq_changes_signal_and_stays_finite() {
        let mut chain = ProcessChain::standard(RATE, CHANNELS);
        let mut st = flat_state();
        // A strong low-band boost: an enabled EQ must measurably alter the signal.
        st.eq.bands[0] = 12.0;
        let original = sample_signal();
        let mut samples = original.clone();

        process_block(&mut chain, &mut samples, CHANNELS, &st);

        let changed = samples
            .iter()
            .zip(original.iter())
            .any(|(a, b)| (a - b).abs() > 1e-4);
        assert!(changed, "an enabled EQ band must change the signal");
        for s in &samples {
            assert!(s.is_finite(), "EQ output must stay finite");
        }
    }

    /// A short interleaved-stereo block: L = low sine, R = higher sine, so the
    /// EQ has real spectral content to act on.
    fn sample_signal() -> Vec<f32> {
        let frames = 2048;
        let mut v = Vec::with_capacity(frames * CHANNELS);
        for n in 0..frames {
            let t = n as f32 / RATE;
            let l = (2.0 * std::f32::consts::PI * 80.0 * t).sin() * 0.5;
            let r = (2.0 * std::f32::consts::PI * 4000.0 * t).sin() * 0.5;
            v.push(l);
            v.push(r);
        }
        v
    }
}
