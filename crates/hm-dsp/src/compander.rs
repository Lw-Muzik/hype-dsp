//! 10-band multiband compander — subtractive (telescoping) crossovers split the
//! signal into 10 bands, each compressed/expanded by an independent dB-domain
//! compressor, then summed. Ported from the mobile Hype MBC (compressor.h +
//! multiband_compressor.h). Global params apply to every band.
//!
//! **Crossover topology — subtractive/telescoping:**
//! Each of the 9 crossovers is a Linkwitz-Riley 4th-order **lowpass only**
//! (two cascaded Butterworth biquads). Band extraction is purely subtractive:
//!
//! ```text
//! rest = input
//! for i in 0..9:
//!     low_i = LP_i(rest)        // uncompressed low portion
//!     rest -= low_i             // remainder = high portion (exact subtraction)
//!     band_i = compress(low_i)
//!     sum += band_i
//! sum += compress(rest)         // final (highest) band
//! ```
//!
//! Because `rest -= low_i` is exact arithmetic, and `Σ low_i + rest_9 = input`
//! by construction (telescoping sum), a flat compander (ratio=1, no gate/makeup)
//! reconstructs the input **exactly** — no comb filtering, no RMS loss.
//!
//! Real-time safe: all band state is pre-allocated in `prepare`; `process` never
//! allocates/locks.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::CompanderState;

pub const BAND_COUNT: usize = 10;
const CROSSOVER_COUNT: usize = BAND_COUNT - 1; // 9
const CENTERS_HZ: [f32; BAND_COUNT] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
const LOG10_20: f32 = 8.685_889; // 20/ln(10)
const INV_LOG10_20: f32 = 0.115_129_255; // ln(10)/20
const GAIN_SMOOTH: f32 = 0.005;

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 {
        0.0
    } else {
        x
    }
}
#[inline]
fn db_to_lin(db: f32) -> f32 {
    (db * INV_LOG10_20).exp()
}
#[inline]
fn lin_to_db(lin: f32) -> f32 {
    if lin < 1e-10 {
        -200.0
    } else {
        lin.ln() * LOG10_20
    }
}

/// One subtractive crossover for one channel: two cascaded Butterworth LP biquads.
/// The high portion is obtained by subtraction (`rest -= low`) — no HP biquads needed.
#[derive(Clone, Copy)]
struct LrChannel {
    lp: [Biquad; 2],
}
impl LrChannel {
    fn new() -> Self {
        Self { lp: [Biquad::identity(); 2] }
    }
    fn configure(&mut self, sr: f32, freq: f32) {
        for b in &mut self.lp {
            b.set_lowpass(sr, freq, BUTTERWORTH_Q);
        }
    }
    fn reset(&mut self) {
        for b in &mut self.lp {
            b.reset();
        }
    }
    /// Return the lowpass output; caller subtracts from its running remainder.
    #[inline]
    fn lowpass(&mut self, x: f32) -> f32 {
        let lp0 = self.lp[0].process_sample(x);
        self.lp[1].process_sample(lp0)
    }
}

/// Per-band single-band compressor/expander (dB-domain), stereo-linked.
struct BandCompressor {
    sample_rate: f32,
    env_db: f32,
    gain_smoothed_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    // cached params
    threshold: f32,
    ratio: f32,
    knee: f32,
    gate: f32,
    expander_ratio: f32,
    makeup_lin: f32,
}
impl BandCompressor {
    fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            sample_rate,
            env_db: -96.0,
            gain_smoothed_db: 0.0,
            attack_coeff: 0.1,
            release_coeff: 0.001,
            threshold: -18.0,
            ratio: 2.5,
            knee: 8.0,
            gate: -70.0,
            expander_ratio: 2.0,
            makeup_lin: 1.0,
        };
        s.recalc(15.0, 45.0);
        s
    }
    fn recalc(&mut self, attack_ms: f32, release_ms: f32) {
        let a = (attack_ms * 0.001).max(0.001);
        let r = (release_ms * 0.001).max(0.001);
        self.attack_coeff = 1.0 - (-1.0 / (a * self.sample_rate)).exp();
        self.release_coeff = 1.0 - (-1.0 / (r * self.sample_rate)).exp();
    }
    fn set_params(&mut self, p: &ProcessorParams) {
        let c = &p.compander;
        self.threshold = c.threshold_db;
        self.ratio = c.ratio.max(1.0);
        self.knee = c.knee_db.max(0.0);
        self.gate = c.gate_db;
        self.expander_ratio = c.expander_ratio.max(1.0);
        self.makeup_lin = db_to_lin(c.makeup_db);
        self.recalc(c.attack_ms, c.release_ms);
    }
    fn reset(&mut self) {
        self.env_db = -96.0;
        self.gain_smoothed_db = 0.0;
    }
    /// dB gain change for an input level (≤0 compression / expansion).
    #[inline]
    fn compute_gain(&self, input_db: f32) -> f32 {
        let mut gain_db = 0.0;
        if input_db < self.gate {
            gain_db = -(self.gate - input_db) * (self.expander_ratio - 1.0);
        }
        if input_db > self.threshold {
            let over = input_db - self.threshold;
            let half_knee = self.knee * 0.5;
            if self.knee > 0.0 && over < half_knee {
                let x = over / half_knee;
                gain_db -= over * (1.0 - 1.0 / self.ratio) * x * 0.5;
            } else {
                let full_over = if self.knee > 0.0 { over - half_knee } else { over };
                if self.knee > 0.0 {
                    gain_db -= half_knee * (1.0 - 1.0 / self.ratio) * 0.5;
                }
                gain_db -= full_over * (1.0 - 1.0 / self.ratio);
            }
        }
        gain_db
    }
    /// Process one stereo frame in place (peak-linked).
    #[inline]
    fn process_frame(&mut self, l: &mut f32, r: &mut f32) {
        let peak = l.abs().max(r.abs());
        let peak_db = lin_to_db(peak);
        if peak_db > self.env_db {
            self.env_db += self.attack_coeff * (peak_db - self.env_db);
        } else {
            self.env_db += self.release_coeff * (peak_db - self.env_db);
        }
        self.env_db = flush(self.env_db + 96.0) - 96.0; // keep env from denormal drift
        let gain_db = self.compute_gain(self.env_db);
        self.gain_smoothed_db += GAIN_SMOOTH * (gain_db - self.gain_smoothed_db);
        let g = db_to_lin(self.gain_smoothed_db) * self.makeup_lin;
        *l *= g;
        *r *= g;
    }
}

/// The 10-band compander stage.
pub struct Compander {
    sample_rate: f32,
    enabled: bool,
    crossovers_l: Vec<LrChannel>, // len CROSSOVER_COUNT
    crossovers_r: Vec<LrChannel>,
    bands: Vec<BandCompressor>, // len BAND_COUNT
    /// Change-guard: last applied compander params so `set_params` can skip the
    /// ≈20 `exp()` coefficient recomputes when nothing changed. Cleared by
    /// `prepare` (sample rate change invalidates rate-dependent coefficients).
    cached: Option<CompanderState>,
}

impl Compander {
    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let mut s = Self {
            sample_rate,
            enabled: false,
            crossovers_l: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            crossovers_r: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            bands: (0..BAND_COUNT).map(|_| BandCompressor::new(sample_rate)).collect(),
            cached: None,
        };
        s.reconfigure();
        s
    }
    fn crossover_freq(i: usize) -> f32 {
        (CENTERS_HZ[i] * CENTERS_HZ[i + 1]).sqrt()
    }
    fn reconfigure(&mut self) {
        for i in 0..CROSSOVER_COUNT {
            let f = Self::crossover_freq(i);
            self.crossovers_l[i].configure(self.sample_rate, f);
            self.crossovers_r[i].configure(self.sample_rate, f);
        }
    }
}

impl AudioProcessor for Compander {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        self.sample_rate = sample_rate;
        for b in &mut self.bands {
            b.sample_rate = sample_rate;
            b.reset();
            // Do NOT call b.recalc here with hardcoded defaults — that would discard
            // the user's attack/release. Invalidating the cache below ensures the
            // next set_params call re-derives coefficients from the real params.
        }
        for c in self.crossovers_l.iter_mut().chain(self.crossovers_r.iter_mut()) {
            c.reset();
        }
        self.reconfigure();
        // Invalidate: sample-rate change makes cached attack/release coefficients stale.
        self.cached = None;
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 {
            return;
        }
        let frames = buffer.len() / channels;
        let stereo = channels >= 2;
        for f in 0..frames {
            let base = f * channels;
            let in_l = buffer[base];
            let in_r = if stereo { buffer[base + 1] } else { in_l };
            // Subtractive (telescoping) crossover:
            //   rest -= low_i  (exact subtraction — no HP biquads)
            //   Σ low_i + rest_9 = input by construction → flat = transparent.
            let mut rest_l = in_l;
            let mut rest_r = in_r;
            let mut sum_l = 0.0_f32;
            let mut sum_r = 0.0_f32;
            for i in 0..CROSSOVER_COUNT {
                // Lowpass the CURRENT remainder (uncompressed) to carve out the band.
                let low_l = self.crossovers_l[i].lowpass(rest_l);
                let low_r = self.crossovers_r[i].lowpass(rest_r);
                // Subtract the uncompressed low from remainder (keeps telescope exact).
                rest_l -= low_l;
                rest_r -= low_r;
                // Compress and accumulate the band.
                let (mut bl, mut br) = (low_l, low_r);
                self.bands[i].process_frame(&mut bl, &mut br);
                sum_l += bl;
                sum_r += br;
            }
            // Final (highest) band = whatever remains after all LP subtractions.
            let (mut bl, mut br) = (rest_l, rest_r);
            self.bands[BAND_COUNT - 1].process_frame(&mut bl, &mut br);
            sum_l += bl;
            sum_r += br;

            let out_l = flush(sum_l).clamp(-4.0, 4.0);
            let out_r = flush(sum_r).clamp(-4.0, 4.0);
            buffer[base] = out_l;
            if stereo {
                buffer[base + 1] = out_r;
            }
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        // `enabled` is read every call — it's a single bool and gates `process`.
        self.enabled = params.compander.enabled;
        // Change-guard: skip the ≈20 exp() coefficient recomputes when state is
        // identical to what we last pushed. `prepare` clears `cached` so a sample-rate
        // change always forces a re-apply even if the user params haven't changed.
        let state = &params.compander;
        if self.cached.as_ref() == Some(state) {
            return;
        }
        for b in &mut self.bands {
            b.set_params(params);
        }
        self.cached = Some(*state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{CompanderState, EngineState};

    fn compander_params(c: CompanderState) -> EngineState {
        EngineState {
            compander: c,
            ..Default::default()
        }
    }

    fn flat_params() -> EngineState {
        compander_params(CompanderState {
            enabled: true,
            ratio: 1.0,
            gate_db: -200.0,
            knee_db: 0.0,
            makeup_db: 0.0,
            threshold_db: -18.0,
            attack_ms: 1.0,
            release_ms: 1.0,
            expander_ratio: 1.0,
        })
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn disabled_is_identity() {
        let mut c = Compander::new(48_000.0, 2);
        // Default state: enabled = false
        c.set_params(&EngineState::default());
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let mut buf = input.clone();
        // interleave as stereo
        let mut stereo: Vec<f32> = buf.iter().flat_map(|&x| [x, x]).collect();
        let orig = stereo.clone();
        c.process(&mut stereo, 2);
        assert_eq!(stereo, orig, "disabled compander must be bit-exact identity");
        // also mono
        let orig2 = buf.clone();
        c.process(&mut buf, 1);
        assert_eq!(buf, orig2, "disabled compander must be bit-exact identity (mono)");
    }

    #[test]
    fn flat_compander_reconstructs_input() {
        // Verify that the subtractive (telescoping) crossover reconstructs the input
        // exactly when ratio=1.0, gate=-200 (no expansion/compression), knee=0,
        // makeup=0. By construction Σ low_i + rest_9 = input, so enabling the
        // compander in flat mode must be transparent: RMS(out - in) / RMS(in) < 2%.
        let sr = 48_000.0_f32;

        // Multi-tone stereo signal covering low/mid/high bands.
        let total_frames = 32_768_usize;
        let signal: Vec<f32> = (0..total_frames)
            .flat_map(|i| {
                let t = i as f32 / sr;
                let s = (2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.3
                    + (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.3
                    + (2.0 * std::f32::consts::PI * 8000.0 * t).sin() * 0.3;
                [s, s * 0.9]
            })
            .collect();

        let mut c = Compander::new(sr, 2);
        c.set_params(&flat_params());

        // Prime a few blocks so filters and envelopes settle.
        let prime_frames = 4096_usize;
        let prime_samples = prime_frames * 2;
        let mut prime_buf = signal[..prime_samples].to_vec();
        c.process(&mut prime_buf, 2);

        // Now process the rest and measure reconstruction error.
        let mut out_buf = signal[prime_samples..].to_vec();
        let in_slice = signal[prime_samples..].to_vec();
        c.process(&mut out_buf, 2);

        // Compute RMS error vs original.
        let err_rms = {
            let sum_sq: f32 = out_buf.iter().zip(in_slice.iter())
                .map(|(o, i)| (o - i) * (o - i))
                .sum();
            (sum_sq / out_buf.len() as f32).sqrt()
        };
        let in_rms = rms(&in_slice);
        let rel_err = err_rms / in_rms;

        assert!(
            rel_err < 0.02,
            "subtractive crossover must reconstruct input with <2% RMS error, \
             got {:.4}% (err_rms={err_rms:.6}, in_rms={in_rms:.6})",
            rel_err * 100.0
        );
    }

    #[test]
    fn loud_input_is_compressed() {
        let sr = 48_000.0_f32;
        let mut c = Compander::new(sr, 2);
        let params = compander_params(CompanderState {
            enabled: true,
            ratio: 8.0,
            threshold_db: -20.0,
            knee_db: 0.0,
            gate_db: -90.0,
            attack_ms: 1.0,
            release_ms: 5.0,
            makeup_db: 0.0,
            expander_ratio: 1.0,
        });
        c.set_params(&params);

        // Loud sustained tone at 0.9 amplitude (well above -20 dB threshold)
        let frames = 4096;
        let signal: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin() * 0.9;
                [s, s]
            })
            .collect();

        // Prime to settle
        let mut prime = signal.clone();
        c.process(&mut prime, 2);

        // Now measure output after settling
        let mut buf = signal.clone();
        c.process(&mut buf, 2);

        let in_peak = signal.iter().map(|x| x.abs()).fold(0.0_f32, f32::max);
        // Take the latter half of output to avoid attack transient
        let out_slice = &buf[buf.len() / 2..];
        let out_peak = out_slice.iter().map(|x| x.abs()).fold(0.0_f32, f32::max);

        assert!(
            out_peak < in_peak * 0.7,
            "compression should reduce level significantly: in_peak={in_peak:.3}, out_peak={out_peak:.3}"
        );
    }

    #[test]
    fn stays_bounded() {
        let sr = 48_000.0_f32;
        let mut c = Compander::new(sr, 2);
        let params = compander_params(CompanderState {
            enabled: true,
            ratio: 2.5,
            threshold_db: -18.0,
            knee_db: 8.0,
            gate_db: -70.0,
            attack_ms: 15.0,
            release_ms: 45.0,
            makeup_db: 6.0,
            expander_ratio: 2.0,
        });
        c.set_params(&params);

        // Hostile: full-scale sustained signal
        let mut buf: Vec<f32> = (0..8192)
            .flat_map(|_| [1.0_f32, -1.0_f32])
            .collect();
        c.process(&mut buf, 2);
        assert!(
            buf.iter().all(|&x| x.abs() <= 4.0),
            "compander output must stay within ±4.0"
        );
    }

    #[test]
    fn quiet_below_gate_is_expanded_down() {
        let sr = 48_000.0_f32;
        let mut c = Compander::new(sr, 2);
        // Gate at -20 dB, input at -40 dB → should be expanded (attenuated) below gate
        let params = compander_params(CompanderState {
            enabled: true,
            ratio: 1.0,
            threshold_db: 0.0,  // no compression above (won't trigger)
            knee_db: 0.0,
            gate_db: -20.0,     // gate at -20 dB
            attack_ms: 1.0,
            release_ms: 1.0,
            makeup_db: 0.0,
            expander_ratio: 4.0, // heavy expansion below gate
        });
        c.set_params(&params);

        // Input at -40 dB (lin ~0.01)
        let amp = 0.01_f32;
        let signal: Vec<f32> = (0..4096)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin() * amp;
                [s, s]
            })
            .collect();

        // Prime to settle
        let mut prime = signal.clone();
        c.process(&mut prime, 2);

        let mut buf = signal.clone();
        c.process(&mut buf, 2);

        let in_rms = rms(&signal[signal.len() / 2..]);
        let out_rms = rms(&buf[buf.len() / 2..]);

        assert!(
            out_rms < in_rms * 0.9,
            "expander should attenuate signal below gate: in_rms={in_rms:.6}, out_rms={out_rms:.6}"
        );
    }
}
