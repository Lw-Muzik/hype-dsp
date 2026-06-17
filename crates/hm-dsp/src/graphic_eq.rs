//! 31-band graphic equalizer: a cascade of peaking biquads at the ISO
//! one-third-octave centers, one independent filter chain per channel.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::{BAND_COUNT, ISO_CENTERS_HZ};

/// Default per-band quality for ISO one-third-octave spacing (≈ 4.318).
pub const DEFAULT_Q: f32 = 4.318;

/// A 31-band graphic EQ. Each channel runs the full cascade independently so
/// stereo imaging is preserved.
pub struct GraphicEq {
    sample_rate: f32,
    channels: usize,
    q: f32,
    enabled: bool,
    pre_gain_lin: f32,
    bands_db: [f32; BAND_COUNT],
    /// `filters[channel][band]` — the per-channel cascade.
    filters: Vec<[Biquad; BAND_COUNT]>,
}

impl GraphicEq {
    /// Create a flat (0 dB) EQ prepared for the given format.
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut eq = Self {
            sample_rate,
            channels,
            q: DEFAULT_Q,
            enabled: true,
            pre_gain_lin: 1.0,
            bands_db: [0.0; BAND_COUNT],
            filters: vec![[Biquad::identity(); BAND_COUNT]; channels.max(1)],
        };
        eq.retune();
        eq
    }

    /// Recompute every band's coefficients from `bands_db` for every channel.
    /// Preserves filter state so re-tuning during playback is click-free.
    fn retune(&mut self) {
        for channel in self.filters.iter_mut() {
            for (band, bq) in channel.iter_mut().enumerate() {
                bq.set_peaking(
                    self.sample_rate,
                    ISO_CENTERS_HZ[band],
                    self.bands_db[band],
                    self.q,
                );
            }
        }
    }
}

impl AudioProcessor for GraphicEq {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.filters = vec![[Biquad::identity(); BAND_COUNT]; self.channels];
        self.retune();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 {
            return;
        }
        for (i, sample) in buffer.iter_mut().enumerate() {
            let channel = i % channels;
            if channel >= self.channels {
                continue;
            }
            let mut x = *sample * self.pre_gain_lin;
            for bq in self.filters[channel].iter_mut() {
                x = bq.process_sample(x);
            }
            *sample = x;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.eq.enabled;
        self.pre_gain_lin = 10f32.powf(params.eq.pre_gain / 20.0);
        if self.bands_db != params.eq.bands {
            self.bands_db = params.eq.bands;
            self.retune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::EngineState;
    use realfft::RealFftPlanner;

    fn flat_params() -> EngineState {
        EngineState::default()
    }

    /// Flat EQ (all bands 0 dB) must return the signal essentially unchanged.
    #[test]
    fn flat_eq_is_identity() {
        let mut eq = GraphicEq::new(48_000.0, 2);
        eq.set_params(&flat_params());

        let input: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.03).sin() * 0.5).collect();
        let mut buffer = input.clone();
        eq.process(&mut buffer, 2);

        for (i, (&y, &x)) in buffer.iter().zip(input.iter()).enumerate() {
            assert!((y - x).abs() < 1e-4, "sample {i}: expected {x}, got {y}");
        }
    }

    /// Magnitude (dB) of the EQ's impulse response at `freq_hz`.
    fn response_db_at(eq: &mut GraphicEq, sample_rate: f32, freq_hz: f32) -> f32 {
        const N: usize = 16_384;
        // Mono impulse response.
        let mut ir = vec![0.0f32; N];
        ir[0] = 1.0;
        eq.process(&mut ir, 1);

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N);
        let mut spectrum = fft.make_output_vec();
        fft.process(&mut ir, &mut spectrum).unwrap();

        let bin = (freq_hz / sample_rate * N as f32).round() as usize;
        let mag = spectrum[bin].norm();
        20.0 * mag.max(1e-12).log10()
    }

    /// Boosting one band raises the magnitude at that center frequency by ~the
    /// requested gain, while a distant frequency is left unchanged.
    #[test]
    fn single_band_boost_is_localized() {
        let sample_rate = 48_000.0;
        // Band index 17 is the 1 kHz ISO center.
        assert_eq!(ISO_CENTERS_HZ[17], 1_000.0);

        let mut params = flat_params();
        params.eq.bands[17] = 6.0;

        let mut eq = GraphicEq::new(sample_rate, 1);
        eq.set_params(&params);
        let at_1k = response_db_at(&mut eq, sample_rate, 1_000.0);

        // Fresh EQ for the distant-frequency probe (reset state).
        let mut eq2 = GraphicEq::new(sample_rate, 1);
        eq2.set_params(&params);
        let at_8k = response_db_at(&mut eq2, sample_rate, 8_000.0);

        assert!(
            (at_1k - 6.0).abs() < 1.5,
            "expected ~+6 dB at 1 kHz, got {at_1k:.2} dB"
        );
        assert!(
            at_8k.abs() < 1.0,
            "expected ~0 dB at 8 kHz, got {at_8k:.2} dB"
        );
    }
}
