//! Per-headphone correction: a preamp plus a cascade of parametric biquads
//! loaded from the active headphone profile (AutoEq ParametricEQ format).
//!
//! Bands are stored in a fixed-size per-channel array so applying a profile in
//! the audio callback never allocates; re-tuning only happens when the band set
//! actually changes.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::ParametricBand;

/// Maximum parametric bands per channel (AutoEq profiles use ≤10).
const MAX_BANDS: usize = 24;

pub struct HeadphoneCorrection {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    preamp_lin: f32,
    active: usize,
    cached: Vec<ParametricBand>,
    filters: Vec<[Biquad; MAX_BANDS]>,
}

impl HeadphoneCorrection {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut h = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            preamp_lin: 1.0,
            active: 0,
            cached: Vec::new(),
            filters: Vec::new(),
        };
        h.reconfigure();
        h
    }

    fn reconfigure(&mut self) {
        self.filters = vec![[Biquad::identity(); MAX_BANDS]; self.channels];
        self.retune();
    }

    fn retune(&mut self) {
        for channel in self.filters.iter_mut() {
            for (b, band) in self.cached.iter().take(MAX_BANDS).enumerate() {
                channel[b].set_from_kind(
                    &band.kind,
                    self.sample_rate,
                    band.freq,
                    band.gain,
                    band.q,
                );
            }
        }
    }
}

impl AudioProcessor for HeadphoneCorrection {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 || (self.active == 0 && self.preamp_lin == 1.0) {
            return;
        }
        for (i, sample) in buffer.iter_mut().enumerate() {
            let c = i % channels;
            if c >= self.channels {
                continue;
            }
            let mut x = *sample * self.preamp_lin;
            for b in 0..self.active {
                x = self.filters[c][b].process_sample(x);
            }
            *sample = x;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.headphone.enabled;
        self.preamp_lin = 10f32.powf(params.headphone.preamp / 20.0);
        if self.cached != params.headphone.bands {
            self.cached = params.headphone.bands.clone();
            self.active = self.cached.len().min(MAX_BANDS);
            self.retune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, HeadphoneCorrectionState};

    #[test]
    fn disabled_is_identity() {
        let mut hc = HeadphoneCorrection::new(48_000.0, 2);
        hc.set_params(&EngineState::default());
        let input = vec![0.2f32, -0.5, 0.3, -0.1];
        let mut buf = input.clone();
        hc.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn applies_parametric_band() {
        let state = EngineState {
            headphone: HeadphoneCorrectionState {
                enabled: true,
                preamp: 0.0,
                bands: vec![ParametricBand {
                    kind: "peaking".into(),
                    freq: 1_000.0,
                    gain: 6.0,
                    q: 1.0,
                }],
            },
            ..Default::default()
        };
        let mut hc = HeadphoneCorrection::new(48_000.0, 1);
        hc.set_params(&state);

        // RMS of a 1 kHz sine should rise with a +6 dB peaking band there.
        let sr = 48_000.0;
        let input: Vec<f32> = (0..4800)
            .map(|n| (n as f32 / sr * 2.0 * std::f32::consts::PI * 1_000.0).sin() * 0.3)
            .collect();
        let mut buf = input.clone();
        hc.process(&mut buf, 1);

        let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        assert!(rms(&buf) > rms(&input) * 1.3, "expected boost at 1 kHz");
    }
}
