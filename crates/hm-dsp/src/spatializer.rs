//! Spatializer: out-of-head imaging for headphones.
//!
//! The baseline combines **stereo widening** (mid/side side-gain) with a gentle
//! **crossfeed** (a low-passed blend of the opposite channel), which together
//! push the image out of the listener's head without collapsing the stereo
//! field. The HRTF path is scaffolded ([`HrtfConvolver`]) for loading a public
//! HRIR set later; until one is loaded, both modes use the baseline.

use crate::{AudioProcessor, ProcessorParams};

const XFEED_LP_HZ: f32 = 700.0;

/// Scaffold for HRTF convolution. No HRIR set is bundled yet, so this reports
/// "not loaded" and the [`Spatializer`] uses its crossfeed/widening baseline.
#[derive(Default)]
pub struct HrtfConvolver {
    loaded: bool,
}

impl HrtfConvolver {
    /// Whether an HRIR set has been loaded (always `false` for now).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
}

pub struct Spatializer {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    amount: f32,
    lp: [f32; 2],
    lp_coeff: f32,
    hrtf: HrtfConvolver,
}

impl Spatializer {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut s = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            amount: 0.0,
            lp: [0.0; 2],
            lp_coeff: 0.0,
            hrtf: HrtfConvolver::default(),
        };
        s.reconfigure();
        s
    }

    fn reconfigure(&mut self) {
        self.lp = [0.0; 2];
        self.lp_coeff = (-2.0 * std::f32::consts::PI * XFEED_LP_HZ / self.sample_rate).exp();
    }

    /// Whether the HRTF path is active (scaffold; currently always baseline).
    pub fn hrtf_loaded(&self) -> bool {
        self.hrtf.is_loaded()
    }
}

impl AudioProcessor for Spatializer {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        // Needs a stereo pair; amount 0 is a no-op.
        if !self.enabled || channels < 2 || self.amount <= 0.0 {
            return;
        }
        let width = 1.0 + self.amount * 0.6;
        let cf = self.amount * 0.2;
        let a = self.lp_coeff;
        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let l = buffer[base];
            let r = buffer[base + 1];

            // Widen via mid/side.
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5 * width;
            let wl = mid + side;
            let wr = mid - side;

            // Crossfeed: each ear gets a low-passed blend of the other.
            self.lp[0] = self.lp[0] * a + wl * (1.0 - a);
            self.lp[1] = self.lp[1] * a + wr * (1.0 - a);
            buffer[base] = wl * (1.0 - cf) + self.lp[1] * cf;
            buffer[base + 1] = wr * (1.0 - cf) + self.lp[0] * cf;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.spatializer.enabled;
        self.amount = params.spatializer.amount.clamp(0.0, 1.0);
        // `mode` is honored once an HRIR set is loaded; until then both modes
        // use the crossfeed/widening baseline.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, SpatialMode, SpatializerState};

    fn stereo(pairs: &[(f32, f32)]) -> Vec<f32> {
        pairs.iter().flat_map(|&(l, r)| [l, r]).collect()
    }

    #[test]
    fn amount_zero_is_identity() {
        let mut sp = Spatializer::new(48_000.0, 2);
        sp.set_params(&EngineState::default()); // disabled, amount 0.5 but disabled
        let input = stereo(&[(0.5, -0.3), (0.2, 0.4)]);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn widens_side_content_when_enabled() {
        let state = EngineState {
            spatializer: SpatializerState {
                enabled: true,
                amount: 1.0,
                mode: SpatialMode::Crossfeed,
            },
            ..Default::default()
        };
        let mut sp = Spatializer::new(48_000.0, 2);
        sp.set_params(&state);

        // A signal with side content; widening should increase |L-R| overall.
        let input = stereo(&[(0.6, 0.1); 64]);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);

        let in_side: f32 = input.chunks(2).map(|c| (c[0] - c[1]).abs()).sum();
        let out_side: f32 = buf.chunks(2).map(|c| (c[0] - c[1]).abs()).sum();
        assert!(
            out_side > in_side,
            "expected wider side: {out_side} vs {in_side}"
        );
        // Bounded (no blow-up).
        assert!(buf.iter().all(|&x| x.abs() < 2.0));
    }
}
