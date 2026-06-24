//! 4× polyphase windowed-sinc oversampler.
//!
//! [`Oversampler4x`] provides alias-free 4× upsampling and downsampling for a
//! **single channel** using a polyphase decomposition of a linear-phase FIR
//! lowpass filter.
//!
//! ## Filter design
//!
//! The prototype FIR runs conceptually at the **4× (oversampled) rate** and
//! passes the original base-band `[0, base_nyquist]` while rejecting everything
//! above it.  At the 4× rate, base Nyquist lives at normalized frequency
//! `0.5 / 4 = 0.125` cycles/sample, so:
//!
//! ```text
//! fc = 0.5 / OVERSAMPLE = 0.125   (cycles per oversampled sample)
//! h[n] = sinc(2·fc·(n − M)) · blackman(n),   M = (NUM_TAPS − 1) / 2
//! ```
//!
//! After normalization to unity DC gain (Σh = 1), those taps are used by the
//! downsampler directly.  The upsampler taps are scaled by `OVERSAMPLE` to
//! compensate for the energy loss inherent in zero-stuffing.
//!
//! ## Polyphase decomposition
//!
//! With `P = OVERSAMPLE = 4` phases and `K = NUM_TAPS / P` taps per phase:
//!
//! ```text
//! h_p[k] = h[P·k + p],   p = 0..P−1,  k = 0..K−1
//! ```
//!
//! **Upsample**: output `y[4n+p] = Σ_k h_p_up[k] · x[n−k]`.  Each of the four
//! phases is a length-K FIR over the *base-rate* input history — no zero
//! multiplies.
//!
//! **Downsample**: for output `y[m]`, push `in4x[4m+p]` into phase p's delay
//! line and accumulate the phase-p FIR; sum over all four phases.
//!
//! Both paths are RT-safe (no allocation after `new`).

use std::f64::consts::PI;

/// Oversampling factor.
pub const OVERSAMPLE: usize = 4;

/// Number of prototype FIR taps (must be a multiple of `OVERSAMPLE`).
///
/// 128 taps with a Blackman window give ≈80 dB stopband attenuation and a
/// transition band narrow enough to suppress a 22 kHz tone at 48 kHz base rate.
const NUM_TAPS: usize = 128;

/// Number of taps in each polyphase branch.
const TAPS_PER_PHASE: usize = NUM_TAPS / OVERSAMPLE; // 32

/// 4× polyphase windowed-sinc oversampler (single channel).
///
/// All state is allocated in [`Oversampler4x::new`]; `upsample` and
/// `downsample` are allocation-free and safe to call from the real-time
/// audio thread.
pub struct Oversampler4x {
    /// Upsampler polyphase taps (`h × OVERSAMPLE`).
    /// Layout: `up_taps[phase * TAPS_PER_PHASE + tap_index]`.
    up_taps: Vec<f32>,
    /// Downsampler polyphase taps (unity DC gain).
    /// Same layout as `up_taps`.
    dn_taps: Vec<f32>,

    /// Shared delay line for the upsampler: stores the last K base-rate input
    /// samples in a circular buffer.
    up_dl: Vec<f32>,
    /// Write position within `up_dl`.
    up_pos: usize,

    /// Per-phase delay lines for the downsampler: each phase has K slots.
    /// Layout: `dn_dl[phase * TAPS_PER_PHASE + pos]`.
    dn_dl: Vec<f32>,
    /// Per-phase write positions within `dn_dl`.
    dn_pos: Vec<usize>,

    /// Half-length M (oversampled samples) — one-way FIR group delay.
    /// M = (NUM_TAPS − 1) / 2 rounded.
    half_len: usize,
}

impl Oversampler4x {
    /// Build the FIR coefficients and allocate all state buffers.
    ///
    /// `sample_rate` is accepted for API symmetry; the FIR is
    /// sample-rate-independent (only the oversampling ratio matters).
    pub fn new(_sample_rate: f32) -> Self {
        // ── 1. Prototype lowpass: windowed-sinc at fc = 0.125 (4× rate) ──────
        let n_taps_f = NUM_TAPS as f64;
        let m = (n_taps_f - 1.0) / 2.0; // centre (63.5 for 128 taps)
        let fc: f64 = 0.5 / OVERSAMPLE as f64; // 0.125

        let mut h = vec![0.0f64; NUM_TAPS];
        for (n, coeff) in h.iter_mut().enumerate() {
            let x = 2.0 * fc * (n as f64 - m);
            let sinc = if x.abs() < 1e-12 { 1.0 } else { (PI * x).sin() / (PI * x) };
            // Blackman window
            let w = 0.42
                - 0.5 * (2.0 * PI * n as f64 / (n_taps_f - 1.0)).cos()
                + 0.08 * (4.0 * PI * n as f64 / (n_taps_f - 1.0)).cos();
            *coeff = sinc * w;
        }

        // ── 2. Normalize to unity DC gain (downsampler taps) ─────────────────
        let dc_gain: f64 = h.iter().sum();
        for v in h.iter_mut() {
            *v /= dc_gain;
        }

        // ── 3. Polyphase decomposition: h_p[k] = h[P*k + p] ─────────────────
        let p = OVERSAMPLE;
        let k = TAPS_PER_PHASE;

        let mut dn_taps = vec![0.0f32; p * k];
        let mut up_taps = vec![0.0f32; p * k];

        for phase in 0..p {
            for tap in 0..k {
                let idx = p * tap + phase; // interleaved order
                // always in range: max idx = 4*31+3 = 127 = NUM_TAPS-1
                dn_taps[phase * k + tap] = h[idx] as f32;
                // Upsampler taps are scaled by P to restore level after
                // zero-stuffing.
                up_taps[phase * k + tap] = (h[idx] * p as f64) as f32;
            }
        }

        // ── 4. State buffers ──────────────────────────────────────────────────
        // Upsampler: one shared circular delay line (base-rate input history).
        let up_dl = vec![0.0f32; k];
        let up_pos = 0usize;

        // Downsampler: one delay line per phase (4× input history per phase).
        let dn_dl = vec![0.0f32; p * k];
        let dn_pos = vec![0usize; p];

        // Group delay at the 4× rate: M ≈ (NUM_TAPS − 1) / 2
        let half_len = m.round() as usize; // 64 for 128 taps

        Self { up_taps, dn_taps, up_dl, up_pos, dn_dl, dn_pos, half_len }
    }

    /// Round-trip group delay **at the base rate** (integer samples).
    ///
    /// Each of the upsample and downsample passes introduces ~M/OVERSAMPLE
    /// base-rate samples of delay (where M is the half-length at the 4× rate).
    /// The total round-trip is `2 * half_len / OVERSAMPLE`, rounded up.
    pub fn latency_samples(&self) -> usize {
        // half_len is one-way group delay at the 4× rate.
        // round-trip at 4× = 2 * half_len
        // convert to base rate (ceiling division)
        (2 * self.half_len).div_ceil(OVERSAMPLE)
    }

    /// Zero all delay-line state.
    pub fn reset(&mut self) {
        self.up_dl.iter_mut().for_each(|v| *v = 0.0);
        self.up_pos = 0;
        self.dn_dl.iter_mut().for_each(|v| *v = 0.0);
        self.dn_pos.iter_mut().for_each(|v| *v = 0);
    }

    /// Upsample `input` by 4× into `out4x`.
    ///
    /// `out4x.len()` must equal `input.len() * OVERSAMPLE`.
    /// No allocation; real-time safe.
    pub fn upsample(&mut self, input: &[f32], out4x: &mut [f32]) {
        debug_assert_eq!(out4x.len(), input.len() * OVERSAMPLE);
        let k = TAPS_PER_PHASE;

        for (n, &x) in input.iter().enumerate() {
            // Push new base-rate sample into the shared circular delay line.
            self.up_dl[self.up_pos] = x;
            let write = self.up_pos;

            // Compute one output sample per phase.
            for phase in 0..OVERSAMPLE {
                let taps = &self.up_taps[phase * k..(phase + 1) * k];
                let mut acc = 0.0f32;
                for (tap_i, &h) in taps.iter().enumerate() {
                    // Circular read: most-recent = write, previous = write-1, …
                    let ri = (write + k - tap_i) % k;
                    acc += h * self.up_dl[ri];
                }
                out4x[n * OVERSAMPLE + phase] = acc;
            }

            // Advance write pointer.
            self.up_pos = (write + 1) % k;
        }
    }

    /// Downsample `in4x` by 4× into `out`.
    ///
    /// `out.len()` must equal `in4x.len() / OVERSAMPLE`.
    /// No allocation; real-time safe.
    pub fn downsample(&mut self, in4x: &[f32], out: &mut [f32]) {
        debug_assert_eq!(out.len(), in4x.len() / OVERSAMPLE);
        let k = TAPS_PER_PHASE;

        for (m_idx, y) in out.iter_mut().enumerate() {
            // The group of 4 oversampled samples for output m_idx is
            // in4x[4m_idx .. 4m_idx+4].  Push each into its phase's delay
            // line and accumulate the phase-p FIR.
            let mut acc = 0.0f32;
            for phase in 0..OVERSAMPLE {
                let sample = in4x[m_idx * OVERSAMPLE + phase];
                let pos = self.dn_pos[phase];

                // Push sample into phase delay line.
                self.dn_dl[phase * k + pos] = sample;

                // FIR over phase delay line.
                let taps = &self.dn_taps[phase * k..(phase + 1) * k];
                for (tap_i, &h) in taps.iter().enumerate() {
                    let ri = (pos + k - tap_i) % k;
                    acc += h * self.dn_dl[phase * k + ri];
                }

                // Advance phase write pointer.
                self.dn_pos[phase] = (pos + 1) % k;
            }
            *y = acc;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn make_sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq / sr * i as f32).sin())
            .collect()
    }

    fn roundtrip(input: &[f32]) -> Vec<f32> {
        let mut ov = Oversampler4x::new(SR);
        let mut up = vec![0.0f32; input.len() * OVERSAMPLE];
        ov.upsample(input, &mut up);
        let mut out = vec![0.0f32; input.len()];
        ov.downsample(&up, &mut out);
        out
    }

    /// `latency_samples()` must be strictly positive.
    #[test]
    fn latency_reported() {
        let ov = Oversampler4x::new(SR);
        assert_eq!(ov.latency_samples(), 32, "round-trip latency for 128-tap 4x; Task 3 dry-delay depends on this");
    }

    /// A 1 kHz sine round-trips with peak amplitude within 3% of input after
    /// discarding the FIR warmup window.
    #[test]
    fn roundtrip_passband_unity() {
        let n = 4096;
        let input = make_sine(1_000.0, SR, n);
        let output = roundtrip(&input);

        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = (lat * 2 + 4).min(n / 2);

        let in_peak = input[skip..].iter().map(|&v| v.abs()).fold(0.0f32, f32::max);
        let out_peak = output[skip..].iter().map(|&v| v.abs()).fold(0.0f32, f32::max);

        let err = (out_peak - in_peak).abs() / in_peak;
        assert!(
            err < 0.03,
            "passband amplitude error {:.2}% (in_peak={in_peak:.4}, out_peak={out_peak:.4})",
            err * 100.0
        );
    }

    /// A constant (DC) input round-trips to ~the same value after settling.
    #[test]
    fn dc_gain_unity() {
        let n = 2048;
        let input = vec![0.5f32; n];
        let output = roundtrip(&input);

        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = (lat * 2 + 8).min(n / 2);

        let tail = &output[skip..];
        let out_avg = tail.iter().copied().sum::<f32>() / tail.len() as f32;
        let err = (out_avg - 0.5).abs();
        assert!(
            err < 0.015,
            "DC gain error {err:.5} (expected 0.5, got {out_avg:.5})"
        );
    }

    /// A 22 kHz tone (in the FIR stopband at 4× rate) must be strongly
    /// attenuated by the round-trip (≥ 20 dB / factor 10×).
    #[test]
    fn near_nyquist_attenuated() {
        let n = 4096;
        let input = make_sine(22_000.0, SR, n);
        let output = roundtrip(&input);

        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = (lat * 2).min(n / 2);

        let in_peak = input[skip..].iter().map(|&v| v.abs()).fold(0.0f32, f32::max);
        let out_peak = output[skip..].iter().map(|&v| v.abs()).fold(0.0f32, f32::max);

        // The Blackman-windowed FIR gives >60 dB stopband attenuation; 10×
        // (−20 dB) is a conservative lower bound.
        assert!(
            out_peak < in_peak * 0.1,
            "22 kHz should be attenuated ≥20 dB; in={in_peak:.4}, out={out_peak:.4}"
        );
    }

    /// Mixed low-frequency content (200/500/1 kHz) RMS is preserved within 5%.
    #[test]
    fn low_freq_energy_preserved() {
        let n = 4096;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                ((2.0 * std::f32::consts::PI * 200.0 * t).sin()
                    + (2.0 * std::f32::consts::PI * 500.0 * t).sin()
                    + (2.0 * std::f32::consts::PI * 1_000.0 * t).sin())
                    / 3.0
            })
            .collect();

        let output = roundtrip(&input);
        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = (lat * 2).min(n / 2);

        let rms = |s: &[f32]| -> f32 {
            (s.iter().map(|&v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        let in_rms = rms(&input[skip..]);
        let out_rms = rms(&output[skip..]);
        let err = (out_rms - in_rms).abs() / in_rms;

        assert!(
            err < 0.05,
            "low-freq RMS error {:.2}% (in={in_rms:.4}, out={out_rms:.4})",
            err * 100.0
        );
    }
}
