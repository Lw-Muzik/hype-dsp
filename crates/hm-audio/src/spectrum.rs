//! Real-time spectrum analysis for the EQ/Enhancer visualizer.
//!
//! The analyzer maintains a rolling Hann-windowed sample window, runs a
//! pre-planned real FFT (allocation-free) on the audio thread, and aggregates
//! the magnitude spectrum into log-spaced bands normalized to `0..1` for
//! display. Bands are published to a lock-free [`SpectrumTap`] (one atomic per
//! band) that the UI-forwarding thread reads — the same pattern as the meters.

use std::sync::atomic::{AtomicU32, Ordering};

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// Number of log-spaced display bands.
pub const SPECTRUM_BANDS: usize = 64;
/// FFT window size (≈ 43 ms at 48 kHz — decent low-frequency resolution).
const FFT_SIZE: usize = 2048;
/// Display dynamic range floor, in dB.
const DB_FLOOR: f32 = 80.0;
const LOW_HZ: f32 = 20.0;
const HIGH_HZ: f32 = 20_000.0;

/// Lock-free latest-spectrum publisher (one `f32`-as-bits atomic per band).
/// Minor tearing across bands is invisible in a moving visualizer, so no
/// seqlock is needed — same trade-off as the meters.
pub struct SpectrumTap {
    bands: [AtomicU32; SPECTRUM_BANDS],
}

impl Default for SpectrumTap {
    fn default() -> Self {
        Self {
            bands: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl SpectrumTap {
    fn store(&self, values: &[f32; SPECTRUM_BANDS]) {
        for (slot, &v) in self.bands.iter().zip(values.iter()) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// Read the latest band magnitudes (normalized `0..1`).
    pub fn load(&self) -> Vec<f32> {
        self.bands
            .iter()
            .map(|b| f32::from_bits(b.load(Ordering::Relaxed)))
            .collect()
    }

    /// Reset all bands to zero (on stop).
    pub fn zero(&self) {
        for b in self.bands.iter() {
            b.store(0, Ordering::Relaxed);
        }
    }
}

/// Owns the FFT plan and rolling state. Created per stream (off the audio
/// thread); its [`push`](Analyzer::push) runs on the audio thread and never
/// allocates.
pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    ring: Vec<f32>,
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    band_bins: [(usize, usize); SPECTRUM_BANDS],
}

impl Analyzer {
    /// Plan the FFT and compute the Hann window and log band edges for the
    /// given sample rate.
    pub fn new(sample_rate: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let fft_in = fft.make_input_vec();
        let fft_out = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();

        // Hann window.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                let x = std::f32::consts::PI * n as f32 / (FFT_SIZE as f32 - 1.0);
                x.sin().powi(2)
            })
            .collect();

        let nyquist = sample_rate * 0.5;
        let high = HIGH_HZ.min(nyquist * 0.99);
        let bins = FFT_SIZE / 2;
        let band_bins = std::array::from_fn(|b| {
            let f_lo = LOW_HZ * (high / LOW_HZ).powf(b as f32 / SPECTRUM_BANDS as f32);
            let f_hi = LOW_HZ * (high / LOW_HZ).powf((b as f32 + 1.0) / SPECTRUM_BANDS as f32);
            let lo = ((f_lo / sample_rate) * FFT_SIZE as f32).round() as usize;
            let hi = ((f_hi / sample_rate) * FFT_SIZE as f32).round() as usize;
            let lo = lo.clamp(1, bins);
            let hi = hi.clamp(lo + 1, bins + 1);
            (lo, hi)
        });

        Self {
            fft,
            window,
            ring: vec![0.0; FFT_SIZE],
            fft_in,
            fft_out,
            scratch,
            band_bins,
        }
    }

    /// Feed a processed interleaved block; updates the rolling window, runs the
    /// FFT, and publishes the new bands to `tap`. Real-time safe.
    pub fn push(&mut self, block: &[f32], channels: usize, tap: &SpectrumTap) {
        if channels == 0 {
            return;
        }
        let frames = block.len() / channels;
        if frames == 0 {
            return;
        }

        // Append a mono downmix of the block to the rolling window.
        if frames >= FFT_SIZE {
            // Only the most recent FFT_SIZE frames matter.
            let start = frames - FFT_SIZE;
            for (i, slot) in self.ring.iter_mut().enumerate() {
                *slot = mono(block, (start + i) * channels, channels);
            }
        } else {
            self.ring.copy_within(frames.., 0);
            let tail = FFT_SIZE - frames;
            for i in 0..frames {
                self.ring[tail + i] = mono(block, i * channels, channels);
            }
        }

        let bands = self.analyze();
        tap.store(&bands);
    }

    /// Run the FFT over the current window and aggregate into normalized bands.
    fn analyze(&mut self) -> [f32; SPECTRUM_BANDS] {
        for i in 0..FFT_SIZE {
            self.fft_in[i] = self.ring[i] * self.window[i];
        }
        // Allocation-free: buffers are pre-sized.
        if self
            .fft
            .process_with_scratch(&mut self.fft_in, &mut self.fft_out, &mut self.scratch)
            .is_err()
        {
            return [0.0; SPECTRUM_BANDS];
        }

        let norm = 2.0 / FFT_SIZE as f32;
        std::array::from_fn(|b| {
            let (lo, hi) = self.band_bins[b];
            let mut peak = 0.0f32;
            for bin in lo..hi {
                let m = self.fft_out[bin].norm() * norm;
                if m > peak {
                    peak = m;
                }
            }
            let db = 20.0 * (peak + 1e-9).log10();
            ((db + DB_FLOOR) / DB_FLOOR).clamp(0.0, 1.0)
        })
    }

    /// Index of the band whose range contains `freq_hz` (for tests/labels).
    pub fn band_for(&self, freq_hz: f32, sample_rate: f32) -> usize {
        let target_bin = ((freq_hz / sample_rate) * FFT_SIZE as f32).round() as usize;
        self.band_bins
            .iter()
            .position(|&(lo, hi)| target_bin >= lo && target_bin < hi)
            .unwrap_or(0)
    }
}

#[inline]
fn mono(block: &[f32], base: usize, channels: usize) -> f32 {
    if channels >= 2 {
        0.5 * (block[base] + block[base + 1])
    } else {
        block[base]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peaks_in_its_band() {
        let sample_rate = 48_000.0;
        let mut analyzer = Analyzer::new(sample_rate);
        let tap = SpectrumTap::default();

        // A 1 kHz sine, as interleaved stereo, longer than the FFT window.
        let freq = 1_000.0f32;
        let frames = FFT_SIZE * 2;
        let mut block = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let s = (n as f32 / sample_rate * 2.0 * std::f32::consts::PI * freq).sin() * 0.8;
            block.push(s);
            block.push(s);
        }
        analyzer.push(&block, 2, &tap);

        let bands = tap.load();
        let argmax = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let expected = analyzer.band_for(freq, sample_rate);

        assert!(
            (argmax as i32 - expected as i32).abs() <= 1,
            "loudest band {argmax} not near the 1 kHz band {expected}"
        );
        // A distant band (≈ 8 kHz) should be much quieter.
        let far = analyzer.band_for(8_000.0, sample_rate);
        assert!(bands[far] < bands[argmax] - 0.1, "8 kHz band not quieter");
    }

    #[test]
    fn silence_produces_zero_bands() {
        let mut analyzer = Analyzer::new(48_000.0);
        let tap = SpectrumTap::default();
        analyzer.push(&vec![0.0; 4096], 2, &tap);
        assert!(tap.load().iter().all(|&v| v < 1e-3));
    }
}
