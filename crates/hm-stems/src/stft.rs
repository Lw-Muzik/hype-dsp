//! Windowed real STFT frames for Spleeter, on `realfft`.
//!
//! The analysis and synthesis window is the **periodic** Hann (the N+1-point
//! symmetric window without its last sample) — the symmetric one differs by one
//! sample and no longer matches training. At hop = NFFT/4 the squared window
//! overlap-adds to the constant [`OLA_GAIN`].

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use crate::config::{NFFT, SPEC_BINS};

/// Sum of squared periodic-Hann windows at 75 % overlap.
pub const OLA_GAIN: f32 = 1.5;

/// The periodic Hann window of length [`NFFT`].
pub fn periodic_hann() -> Vec<f32> {
    (0..NFFT)
        .map(|n| (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / NFFT as f64).cos()) as f32)
        .collect()
}

/// One channel's forward/inverse transform with its scratch buffers.
pub struct Stft {
    window: Vec<f32>,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    frame: Vec<f32>,
    fwd_scratch: Vec<Complex32>,
    inv_scratch: Vec<Complex32>,
    inv_spec: Vec<Complex32>,
}

impl Default for Stft {
    fn default() -> Self {
        Self::new()
    }
}

impl Stft {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(NFFT);
        let inv = planner.plan_fft_inverse(NFFT);
        Self {
            window: periodic_hann(),
            frame: vec![0.0; NFFT],
            fwd_scratch: fwd.make_scratch_vec(),
            inv_scratch: inv.make_scratch_vec(),
            inv_spec: vec![Complex32::new(0.0, 0.0); SPEC_BINS],
            fwd,
            inv,
        }
    }

    pub fn window(&self) -> &[f32] {
        &self.window
    }

    /// Transform `NFFT` raw samples (the analysis window is applied here) into
    /// [`SPEC_BINS`] complex bins.
    pub fn forward(&mut self, samples: &[f32], spec: &mut [Complex32]) {
        debug_assert_eq!(samples.len(), NFFT);
        debug_assert_eq!(spec.len(), SPEC_BINS);
        for ((f, &s), &w) in self.frame.iter_mut().zip(samples).zip(&self.window) {
            *f = s * w;
        }
        self.fwd
            .process_with_scratch(&mut self.frame, spec, &mut self.fwd_scratch)
            .expect("fft sizes are fixed");
    }

    /// Inverse of a Hermitian half spectrum back to `NFFT` samples with the
    /// synthesis window applied. The caller overlap-adds and divides by
    /// [`OLA_GAIN`].
    pub fn inverse(&mut self, spec: &[Complex32], out: &mut [f32]) {
        debug_assert_eq!(spec.len(), SPEC_BINS);
        debug_assert_eq!(out.len(), NFFT);
        self.inv_spec.copy_from_slice(spec);
        // A real signal's DC and Nyquist bins are real.
        self.inv_spec[0].im = 0.0;
        self.inv_spec[SPEC_BINS - 1].im = 0.0;
        self.inv
            .process_with_scratch(&mut self.inv_spec, out, &mut self.inv_scratch)
            .expect("fft sizes are fixed");
        let scale = 1.0 / NFFT as f32;
        for (o, &w) in out.iter_mut().zip(&self.window) {
            *o *= scale * w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HOP;

    #[test]
    fn window_is_periodic_not_symmetric() {
        let w = periodic_hann();
        assert_eq!(w[0], 0.0);
        // Periodic: peak is exactly 1 at N/2 and the window is NOT symmetric
        // about its midpoint sample index (N-1)/2.
        assert!((w[NFFT / 2] - 1.0).abs() < 1e-7);
        assert!((w[1] - w[NFFT - 1]).abs() < 1e-7, "w[n] == w[N-n]");
        assert!(w[NFFT - 1] > 0.0 && w[NFFT - 1] < 1e-5);
        // numpy: np.hanning(N+1)[:-1][1]
        let expect = 0.5 - 0.5 * (std::f64::consts::TAU / NFFT as f64).cos();
        assert!((f64::from(w[1]) - expect).abs() < 1e-9);
    }

    #[test]
    fn squared_windows_overlap_add_to_the_ola_gain() {
        let w = periodic_hann();
        for n in 0..HOP {
            let s: f32 = (0..NFFT / HOP).map(|k| w[n + k * HOP].powi(2)).sum();
            assert!((s - OLA_GAIN).abs() < 1e-5, "n={n} sum={s}");
        }
    }

    #[test]
    fn forward_inverse_overlap_add_reconstructs_the_signal() {
        let mut stft = Stft::new();
        let total = NFFT + HOP * 12;
        let x: Vec<f32> = (0..total)
            .map(|i| (i as f32 * 0.01).sin() * 0.5 + (i as f32 * 0.37).cos() * 0.2)
            .collect();
        let mut acc = vec![0.0f32; total];
        let mut spec = vec![Complex32::new(0.0, 0.0); SPEC_BINS];
        let mut frame = vec![0.0f32; NFFT];
        let mut at = 0;
        while at + NFFT <= total {
            stft.forward(&x[at..at + NFFT], &mut spec);
            stft.inverse(&spec, &mut frame);
            for (a, f) in acc[at..at + NFFT].iter_mut().zip(&frame) {
                *a += f;
            }
            at += HOP;
        }
        // Fully covered region: [NFFT - HOP, total - (NFFT - HOP)).
        for i in (NFFT - HOP)..(total - (NFFT - HOP)) {
            let y = acc[i] / OLA_GAIN;
            assert!((y - x[i]).abs() < 1e-4, "i={i} y={y} x={}", x[i]);
        }
    }
}
