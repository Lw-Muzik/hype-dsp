//! Bass boost: a low-shelf biquad with an optional gentle harmonic
//! enhancement that adds saturated low-frequency content to imply bass on small
//! drivers (where the fundamental can't be reproduced).

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};

const SHELF_HZ: f32 = 110.0;
const SHELF_Q: f32 = 0.707;
const HARMONIC_LP_HZ: f32 = 160.0;

pub struct BassBoost {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    harmonics: bool,
    amount_db: f32,
    shelves: Vec<Biquad>,
    lp_state: Vec<f32>,
    lp_coeff: f32,
}

impl BassBoost {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut b = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            harmonics: false,
            amount_db: 0.0,
            shelves: Vec::new(),
            lp_state: Vec::new(),
            lp_coeff: 0.0,
        };
        b.reconfigure();
        b
    }

    fn reconfigure(&mut self) {
        self.shelves = vec![Biquad::identity(); self.channels];
        self.lp_state = vec![0.0; self.channels];
        self.lp_coeff = (-2.0 * std::f32::consts::PI * HARMONIC_LP_HZ / self.sample_rate).exp();
        self.retune();
    }

    fn retune(&mut self) {
        for shelf in self.shelves.iter_mut() {
            shelf.set_low_shelf(self.sample_rate, SHELF_HZ, self.amount_db, SHELF_Q);
        }
    }
}

impl AudioProcessor for BassBoost {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 {
            return;
        }
        let harm_gain = if self.harmonics { 0.35 } else { 0.0 };
        for (i, sample) in buffer.iter_mut().enumerate() {
            let c = i % channels;
            if c >= self.channels {
                continue;
            }
            let x = *sample;
            let mut y = self.shelves[c].process_sample(x);
            if harm_gain > 0.0 {
                let lp = self.lp_state[c] * self.lp_coeff + x * (1.0 - self.lp_coeff);
                self.lp_state[c] = lp;
                // tanh adds odd harmonics (DC-free), reinforcing perceived bass.
                y += harm_gain * (lp * 2.0).tanh() * 0.5;
            }
            *sample = y;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.bass.enabled;
        self.harmonics = params.bass.harmonics;
        if (self.amount_db - params.bass.amount).abs() > f32::EPSILON {
            self.amount_db = params.bass.amount;
            self.retune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{BassBoostState, EngineState};

    #[test]
    fn disabled_is_identity() {
        let mut bass = BassBoost::new(48_000.0, 2);
        bass.set_params(&EngineState::default()); // bass disabled by default
        let input = vec![0.3f32, -0.4, 0.1, -0.2];
        let mut buf = input.clone();
        bass.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn enabled_boosts_low_frequencies() {
        let state = EngineState {
            bass: BassBoostState {
                enabled: true,
                amount: 6.0,
                harmonics: false,
            },
            ..Default::default()
        };
        let mut bass = BassBoost::new(48_000.0, 1);
        bass.set_params(&state);
        // Steady (near-DC) input should be amplified by the low shelf.
        let mut y = 0.0;
        for _ in 0..3000 {
            y = {
                let mut s = [1.0f32];
                bass.process(&mut s, 1);
                s[0]
            };
        }
        assert!(y > 1.4, "expected low-shelf boost, got {y}");
    }
}
