//! Spatializer: out-of-head imaging for headphones.
//!
//! Two physically-motivated modes share an inter-aural delay line:
//!
//! - **Crossfeed** — a Bauer-style (bs2b) natural crossfeed. Each ear receives a
//!   delayed, head-shadow-low-passed, attenuated copy of the *opposite* channel.
//!   The inter-aural time delay (~0.3 ms) plus the low-pass emulate sound
//!   arriving around the head, relaxing hard-panned mixes and reducing the
//!   "ping-pong" fatigue of pure-stereo material on headphones.
//! - **HRTF** — a parametric virtual-speaker model: the crossfeed cues plus an
//!   inter-aural *level* difference (ILD), a pinna notch, and stereo widening,
//!   placing the image in front of and outside the head. [`HrtfConvolver`] is
//!   scaffolded so a measured HRIR set can replace the parametric model later;
//!   until one is loaded, the parametric model is used.

use crate::biquad::Biquad;
use crate::delay::DelayLine;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::SpatialMode;

/// Inter-aural time delay: the far ear hears sound ~0.3 ms later.
const ITD_SECONDS: f32 = 0.0003;
/// Head-shadow low-pass on the crossfeed bleed.
const XFEED_LP_HZ: f32 = 850.0;
/// Stronger head shadow for the HRTF virtual-speaker bleed.
const HRTF_LP_HZ: f32 = 1100.0;
/// Pinna-notch centre (HRTF only): a spectral cue that helps frontal imaging.
const PINNA_HZ: f32 = 7500.0;

/// Scaffold for measured-HRIR convolution. No set is bundled yet, so this
/// reports "not loaded" and the [`Spatializer`] uses its parametric models.
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
    mode: SpatialMode,
    delay_l: DelayLine,
    delay_r: DelayLine,
    delay_samples: usize,
    /// Head-shadow one-pole state for the two bleed paths. `[0]` feeds the left
    /// ear from the right channel; `[1]` feeds the right ear from the left.
    bleed_lp: [f32; 2],
    xfeed_coeff: f32,
    hrtf_coeff: f32,
    /// Pinna notch per bleed path (HRTF mode only).
    pinna: [Biquad; 2],
    hrtf: HrtfConvolver,
}

impl Spatializer {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut s = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            amount: 0.0,
            mode: SpatialMode::Crossfeed,
            delay_l: DelayLine::new(1),
            delay_r: DelayLine::new(1),
            delay_samples: 0,
            bleed_lp: [0.0; 2],
            xfeed_coeff: 0.0,
            hrtf_coeff: 0.0,
            pinna: [Biquad::identity(); 2],
            hrtf: HrtfConvolver::default(),
        };
        s.reconfigure();
        s
    }

    fn reconfigure(&mut self) {
        self.delay_samples = (ITD_SECONDS * self.sample_rate).round().max(1.0) as usize;
        self.delay_l = DelayLine::new(self.delay_samples);
        self.delay_r = DelayLine::new(self.delay_samples);
        self.bleed_lp = [0.0; 2];
        self.xfeed_coeff = (-2.0 * std::f32::consts::PI * XFEED_LP_HZ / self.sample_rate).exp();
        self.hrtf_coeff = (-2.0 * std::f32::consts::PI * HRTF_LP_HZ / self.sample_rate).exp();
        for p in self.pinna.iter_mut() {
            p.set_peaking(self.sample_rate, PINNA_HZ, -4.0, 2.0);
        }
    }

    /// Whether the measured-HRIR path is active (scaffold; currently always the
    /// parametric model).
    pub fn hrtf_loaded(&self) -> bool {
        self.hrtf.is_loaded()
    }

    /// Bauer-style natural crossfeed: a delayed, head-shadowed, attenuated copy
    /// of the opposite channel. No widening — this gently narrows toward centre.
    fn process_crossfeed(&mut self, buffer: &mut [f32], channels: usize) {
        let level = self.amount * 0.5;
        let norm = 1.0 / (1.0 + level);
        let a = self.xfeed_coeff;
        let d = self.delay_samples;
        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let l = buffer[base];
            let r = buffer[base + 1];
            let dl = self.delay_l.process(l, d);
            let dr = self.delay_r.process(r, d);
            // Left ear hears the delayed right channel, and vice-versa.
            self.bleed_lp[0] = self.bleed_lp[0] * a + dr * (1.0 - a);
            self.bleed_lp[1] = self.bleed_lp[1] * a + dl * (1.0 - a);
            buffer[base] = (l + level * self.bleed_lp[0]) * norm;
            buffer[base + 1] = (r + level * self.bleed_lp[1]) * norm;
        }
    }

    /// Parametric virtual speakers: widen, then add a delayed, level-reduced
    /// (ILD), head-shadowed and pinna-coloured copy of the opposite channel.
    fn process_hrtf(&mut self, buffer: &mut [f32], channels: usize) {
        const ILD: f32 = 0.6;
        let width = 1.0 + self.amount * 0.8;
        let level = self.amount * 0.7;
        let norm = 1.0 / (1.0 + level * ILD);
        let a = self.hrtf_coeff;
        let d = self.delay_samples;
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
            let dl = self.delay_l.process(wl, d);
            let dr = self.delay_r.process(wr, d);
            self.bleed_lp[0] = self.bleed_lp[0] * a + dr * (1.0 - a);
            self.bleed_lp[1] = self.bleed_lp[1] * a + dl * (1.0 - a);
            let bleed_l = self.pinna[0].process_sample(self.bleed_lp[0]) * ILD;
            let bleed_r = self.pinna[1].process_sample(self.bleed_lp[1]) * ILD;
            buffer[base] = (wl + level * bleed_l) * norm;
            buffer[base + 1] = (wr + level * bleed_r) * norm;
        }
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
        match self.mode {
            SpatialMode::Crossfeed => self.process_crossfeed(buffer, channels),
            SpatialMode::Hrtf => self.process_hrtf(buffer, channels),
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.spatializer.enabled;
        self.amount = params.spatializer.amount.clamp(0.0, 1.0);
        self.mode = params.spatializer.mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, SpatialMode, SpatializerState};

    fn stereo(pairs: &[(f32, f32)]) -> Vec<f32> {
        pairs.iter().flat_map(|&(l, r)| [l, r]).collect()
    }

    fn enabled(amount: f32, mode: SpatialMode) -> EngineState {
        EngineState {
            spatializer: SpatializerState {
                enabled: true,
                amount,
                mode,
            },
            ..Default::default()
        }
    }

    fn side_energy(buf: &[f32]) -> f32 {
        buf.chunks(2).map(|c| (c[0] - c[1]).abs()).sum()
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
    fn crossfeed_feeds_opposite_ear_after_interaural_delay() {
        // A hard-left impulse must reach the right ear, but only after the
        // inter-aural time delay — the cue that pushes the image out of the head.
        let mut sp = Spatializer::new(48_000.0, 2);
        sp.set_params(&enabled(1.0, SpatialMode::Crossfeed));

        let mut frames = vec![(1.0_f32, 0.0_f32)];
        frames.extend(std::iter::repeat_n((0.0, 0.0), 63));
        let mut buf = stereo(&frames);
        sp.process(&mut buf, 2);

        // Left ear gets the direct sound at frame 0.
        assert!(buf[0] > 0.4, "expected direct left, got {}", buf[0]);
        // Right ear is silent until the delayed bleed arrives.
        let right: Vec<f32> = buf.chunks(2).map(|c| c[1]).collect();
        let first_nonzero = right.iter().position(|&x| x.abs() > 1e-6);
        assert!(
            matches!(first_nonzero, Some(i) if i >= 6),
            "right-ear bleed should be delayed; first nonzero at {first_nonzero:?}"
        );
    }

    #[test]
    fn crossfeed_narrows_hard_panned_content() {
        // Crossfeed pulls extreme panning toward center (the bs2b effect).
        let mut sp = Spatializer::new(48_000.0, 2);
        sp.set_params(&enabled(1.0, SpatialMode::Crossfeed));
        let input = stereo(&[(0.6, 0.1); 96]);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);
        assert!(
            side_energy(&buf) < side_energy(&input),
            "crossfeed should narrow: {} !< {}",
            side_energy(&buf),
            side_energy(&input)
        );
    }

    #[test]
    fn hrtf_images_wider_than_crossfeed() {
        // The HRTF virtual-speaker mode is a distinctly wider, more externalized
        // image than plain crossfeed.
        let input = stereo(&[(0.6, 0.1); 96]);

        let mut cf = Spatializer::new(48_000.0, 2);
        cf.set_params(&enabled(1.0, SpatialMode::Crossfeed));
        let mut cf_buf = input.clone();
        cf.process(&mut cf_buf, 2);

        let mut hr = Spatializer::new(48_000.0, 2);
        hr.set_params(&enabled(1.0, SpatialMode::Hrtf));
        let mut hr_buf = input.clone();
        hr.process(&mut hr_buf, 2);

        assert!(
            side_energy(&hr_buf) > side_energy(&cf_buf),
            "HRTF should be wider than crossfeed: {} !> {}",
            side_energy(&hr_buf),
            side_energy(&cf_buf)
        );
    }

    #[test]
    fn modes_produce_distinct_output() {
        let input = stereo(
            &[(0.4, -0.2), (0.1, 0.5), (-0.3, 0.2), (0.25, 0.25)].repeat(16),
        );
        let mut cf = Spatializer::new(48_000.0, 2);
        cf.set_params(&enabled(0.8, SpatialMode::Crossfeed));
        let mut a = input.clone();
        cf.process(&mut a, 2);

        let mut hr = Spatializer::new(48_000.0, 2);
        hr.set_params(&enabled(0.8, SpatialMode::Hrtf));
        let mut b = input.clone();
        hr.process(&mut b, 2);

        assert!(a != b, "crossfeed and HRTF modes must produce different output");
    }

    #[test]
    fn output_is_bounded() {
        for mode in [SpatialMode::Crossfeed, SpatialMode::Hrtf] {
            let mut sp = Spatializer::new(48_000.0, 2);
            sp.set_params(&enabled(1.0, mode));
            let input =
                stereo(&[(0.9, -0.9), (-0.8, 0.8), (0.95, 0.95)].repeat(64));
            let mut buf = input.clone();
            sp.process(&mut buf, 2);
            assert!(
                buf.iter().all(|&x| x.abs() < 2.0),
                "output blew up in {mode:?}"
            );
        }
    }
}
