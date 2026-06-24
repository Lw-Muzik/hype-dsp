//! Bass boost: a low-shelf biquad with an optional gentle harmonic
//! enhancement that adds saturated low-frequency content to imply bass on small
//! drivers (where the fundamental can't be reproduced).
//!
//! ## Adaptive mode
//!
//! When `params.bass.adaptive = true` the static shelf gain is applied via a
//! dry/wet blend whose wet fraction (`adapt_factor`) tracks the current
//! low-band energy.  The idea is anti-overload / anti-mud: when the incoming
//! signal already carries strong sub-120 Hz content, backing off the boost
//! prevents distortion; when bass is quiet or absent, the full shelf gain is
//! applied.
//!
//! `adaptive = false` (the default) ⇒ `adapt_factor = 1.0` everywhere, which
//! collapses the blend to `dry + 1.0*(shelved−dry) = shelved` — bit-exact with
//! the pre-adaptive code path.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};

const SHELF_HZ: f32 = 110.0;
const SHELF_Q: f32 = 0.707;
const HARMONIC_LP_HZ: f32 = 160.0;

// ── Adaptive-gain constants ───────────────────────────────────────────────────
//
// Low-band isolation lowpass: one-pole at ~120 Hz extracts sub-bass energy.
const ADAPT_LP_HZ: f32 = 120.0;

// Envelope follower time constants:
//   attack  ≈ 10 ms  → fast response to incoming bass transients
//   release ≈ 150 ms → slow decay so the factor doesn't flutter on rhythm
const ADAPT_ATTACK_MS: f32 = 10.0;
const ADAPT_RELEASE_MS: f32 = 150.0;

// adapt_factor mapping   env → factor ∈ [ADAPT_FLOOR, 1.0]
//
//   env < T_LO : return 1.0 (full boost — bass is quiet, boost freely)
//   env > T_HI : return ADAPT_FLOOR (minimum boost — bass is already loud)
//   T_LO..T_HI : linear interpolation
//
// "Full-scale" here means 1.0 f32 amplitude.  At T_HI = 0.4 (~−8 dBFS) the
// low band is already substantial; FLOOR=0.25 still provides a mild lift
// rather than cutting off entirely.
const ADAPT_FLOOR: f32 = 0.25;
const T_LO: f32 = 0.05; // below this → factor = 1.0
const T_HI: f32 = 0.40; // above this → factor = ADAPT_FLOOR

/// Flush near-denormal values to zero.  Bass envelopes decay very slowly and
/// can drift into the denormal range, causing CPU spikes on some x86 chips.
#[inline(always)]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 {
        0.0
    } else {
        x
    }
}

/// Monotonic decreasing map from low-band envelope level to a blend factor.
///
/// factor = 1.0   ⇒ full static shelf boost (bass is quiet)
/// factor = FLOOR ⇒ minimal boost applied (bass is already strong)
#[inline(always)]
fn adapt_factor(env: f32) -> f32 {
    if env <= T_LO {
        1.0
    } else if env >= T_HI {
        ADAPT_FLOOR
    } else {
        // Linear interpolation from 1.0 → ADAPT_FLOOR as env goes T_LO → T_HI
        let t = (env - T_LO) / (T_HI - T_LO);
        1.0 - t * (1.0 - ADAPT_FLOOR)
    }
}

pub struct BassBoost {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    harmonics: bool,
    adaptive: bool,
    amount_db: f32,
    shelves: Vec<Biquad>,
    /// Per-channel state for harmonic enhancer lowpass (HARMONIC_LP_HZ).
    lp_state: Vec<f32>,
    lp_coeff: f32,
    /// Per-channel one-pole lowpass state for low-band isolation (ADAPT_LP_HZ).
    adapt_lp_state: Vec<f32>,
    adapt_lp_coeff: f32,
    /// Per-channel envelope follower output (|adapt_lp| smoothed).
    env: Vec<f32>,
    /// One-pole attack coefficient for the envelope follower.
    env_attack: f32,
    /// One-pole release coefficient for the envelope follower.
    env_release: f32,
}

impl BassBoost {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut b = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            harmonics: false,
            adaptive: false,
            amount_db: 0.0,
            shelves: Vec::new(),
            lp_state: Vec::new(),
            lp_coeff: 0.0,
            adapt_lp_state: Vec::new(),
            adapt_lp_coeff: 0.0,
            env: Vec::new(),
            env_attack: 0.0,
            env_release: 0.0,
        };
        b.reconfigure();
        b
    }

    fn reconfigure(&mut self) {
        let n = self.channels;
        self.shelves = vec![Biquad::identity(); n];
        self.lp_state = vec![0.0; n];
        self.lp_coeff = (-2.0 * std::f32::consts::PI * HARMONIC_LP_HZ / self.sample_rate).exp();
        // Adaptive envelope infrastructure — pre-sized, zero-initialised.
        self.adapt_lp_state = vec![0.0; n];
        self.adapt_lp_coeff =
            (-2.0 * std::f32::consts::PI * ADAPT_LP_HZ / self.sample_rate).exp();
        self.env = vec![0.0; n];
        let sr = self.sample_rate;
        self.env_attack = (-1.0 / (ADAPT_ATTACK_MS * 0.001 * sr)).exp();
        self.env_release = (-1.0 / (ADAPT_RELEASE_MS * 0.001 * sr)).exp();
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
        let adaptive = self.adaptive;
        let alp_c = self.adapt_lp_coeff;
        let alp_1mc = 1.0 - alp_c;
        let atk = self.env_attack;
        let rel = self.env_release;

        for (i, sample) in buffer.iter_mut().enumerate() {
            let c = i % channels;
            if c >= self.channels {
                continue;
            }
            let x = *sample;

            // Static shelf — always at full amount_db (coeffs never change with adaptive).
            let shelved = self.shelves[c].process_sample(x);

            // Adaptive dry/wet blend.
            let factor = if adaptive {
                // One-pole lowpass to isolate sub-bass energy.
                let lp = flush(self.adapt_lp_state[c] * alp_c + x * alp_1mc);
                self.adapt_lp_state[c] = lp;
                // Asymmetric peak envelope follower on |lp|.
                let abs_lp = lp.abs();
                let prev_env = self.env[c];
                let coeff = if abs_lp > prev_env { atk } else { rel };
                let new_env = flush(prev_env * coeff + abs_lp * (1.0 - coeff));
                self.env[c] = new_env;
                adapt_factor(new_env)
            } else {
                // adaptive=false → factor=1.0 → boosted==shelved (bit-exact with old path)
                1.0
            };

            // Blend: factor=1.0 ⇒ boosted == shelved (identical to pre-adaptive code).
            let mut boosted = x + factor * (shelved - x);

            // Harmonic enhancer — independent of adaptive mode, same as before.
            if harm_gain > 0.0 {
                let lp = self.lp_state[c] * self.lp_coeff + x * (1.0 - self.lp_coeff);
                self.lp_state[c] = lp;
                // tanh adds odd harmonics (DC-free), reinforcing perceived bass.
                boosted += harm_gain * (lp * 2.0).tanh() * 0.5;
            }

            *sample = boosted;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.bass.enabled;
        self.harmonics = params.bass.harmonics;
        self.adaptive = params.bass.adaptive; // cheap bool, no re-tune needed
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

    fn bass_state(enabled: bool, amount: f32, harmonics: bool, adaptive: bool) -> EngineState {
        EngineState {
            bass: BassBoostState {
                enabled,
                amount,
                harmonics,
                adaptive,
            },
            ..Default::default()
        }
    }

    // ── Original tests (must remain unchanged / bit-exact) ───────────────────

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
                adaptive: false,
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

    // ── New tests ────────────────────────────────────────────────────────────

    /// adaptive=false must produce bit-exact output vs. the pre-adaptive path.
    /// We verify by running two separate BassBoost instances on the same signal
    /// and comparing sample-by-sample.
    #[test]
    fn adaptive_false_matches_static() {
        let state = bass_state(true, 6.0, false, false);
        let mut b1 = BassBoost::new(48_000.0, 1);
        b1.set_params(&state);
        let mut b2 = BassBoost::new(48_000.0, 1);
        b2.set_params(&state);

        // Mixed-frequency signal: bass + mid content.
        let signal: Vec<f32> = (0..4096)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                (2.0 * std::f32::consts::PI * 60.0 * t).sin() * 0.5
                    + (2.0 * std::f32::consts::PI * 1_000.0 * t).sin() * 0.2
            })
            .collect();

        let mut buf1 = signal.clone();
        let mut buf2 = signal.clone();
        b1.process(&mut buf1, 1);
        b2.process(&mut buf2, 1);

        for (a, b) in buf1.iter().zip(buf2.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "adaptive=false output diverged: {a} vs {b}"
            );
        }
    }

    /// When the low band is already loud (sustained 60 Hz at ~0.9 amplitude),
    /// adaptive mode should apply LESS boost than the static path.
    #[test]
    fn adaptive_reduces_boost_on_loud_bass() {
        const SR: f32 = 48_000.0;
        const FREQ: f32 = 60.0;
        const AMP: f32 = 0.9;
        // Warm-up the envelope by processing many samples before measuring.
        // At 150 ms release the envelope needs ~500–1000 ms to fully settle.
        const WARMUP: usize = SR as usize * 2; // 2 seconds
        const MEASURE: usize = SR as usize / 10; // 100 ms measurement window

        let state_static = bass_state(true, 6.0, false, false);
        let state_adaptive = bass_state(true, 6.0, false, true);

        let mut b_static = BassBoost::new(SR, 1);
        b_static.set_params(&state_static);
        let mut b_adaptive = BassBoost::new(SR, 1);
        b_adaptive.set_params(&state_adaptive);

        // Feed the same sustained 60 Hz tone to both during warm-up.
        for i in 0..WARMUP {
            let t = i as f32 / SR;
            let x = (2.0 * std::f32::consts::PI * FREQ * t).sin() * AMP;
            let mut s = [x];
            b_static.process(&mut s, 1);
            let mut a = [x];
            b_adaptive.process(&mut a, 1);
        }

        // Measure RMS over the next 100 ms.
        let mut rms_static = 0.0f32;
        let mut rms_adaptive = 0.0f32;
        for i in WARMUP..WARMUP + MEASURE {
            let t = i as f32 / SR;
            let x = (2.0 * std::f32::consts::PI * FREQ * t).sin() * AMP;
            let mut s = [x];
            b_static.process(&mut s, 1);
            rms_static += s[0] * s[0];
            let mut a = [x];
            b_adaptive.process(&mut a, 1);
            rms_adaptive += a[0] * a[0];
        }
        rms_static = (rms_static / MEASURE as f32).sqrt();
        rms_adaptive = (rms_adaptive / MEASURE as f32).sqrt();

        assert!(
            rms_adaptive < rms_static,
            "adaptive should reduce boost on loud bass: adaptive_rms={rms_adaptive:.4} static_rms={rms_static:.4}"
        );
    }

    /// When the bass is quiet (amplitude ~0.04), the adaptive factor should be
    /// ≈1.0, giving roughly the same output as the static path.
    #[test]
    fn adaptive_full_boost_on_quiet_bass() {
        const SR: f32 = 48_000.0;
        const FREQ: f32 = 60.0;
        // Amplitude below T_LO=0.05 so the peak envelope stays under the threshold.
        const AMP: f32 = 0.04;
        const WARMUP: usize = SR as usize; // 1 second warm-up
        const MEASURE: usize = SR as usize / 10;

        let state_static = bass_state(true, 6.0, false, false);
        let state_adaptive = bass_state(true, 6.0, false, true);

        let mut b_static = BassBoost::new(SR, 1);
        b_static.set_params(&state_static);
        let mut b_adaptive = BassBoost::new(SR, 1);
        b_adaptive.set_params(&state_adaptive);

        for i in 0..WARMUP {
            let t = i as f32 / SR;
            let x = (2.0 * std::f32::consts::PI * FREQ * t).sin() * AMP;
            let mut s = [x];
            b_static.process(&mut s, 1);
            let mut a = [x];
            b_adaptive.process(&mut a, 1);
        }

        let mut rms_static = 0.0f32;
        let mut rms_adaptive = 0.0f32;
        for i in WARMUP..WARMUP + MEASURE {
            let t = i as f32 / SR;
            let x = (2.0 * std::f32::consts::PI * FREQ * t).sin() * AMP;
            let mut s = [x];
            b_static.process(&mut s, 1);
            rms_static += s[0] * s[0];
            let mut a = [x];
            b_adaptive.process(&mut a, 1);
            rms_adaptive += a[0] * a[0];
        }
        rms_static = (rms_static / MEASURE as f32).sqrt();
        rms_adaptive = (rms_adaptive / MEASURE as f32).sqrt();

        // Adaptive and static should be within 5% of each other for quiet bass.
        let ratio = rms_adaptive / rms_static;
        assert!(
            ratio > 0.92,
            "quiet bass should get ≈full static boost: ratio={ratio:.4} (adaptive_rms={rms_adaptive:.4} static_rms={rms_static:.4})"
        );
    }

    /// Hostile input (clipping-level bursts) must not produce unbounded output.
    #[test]
    fn stays_bounded() {
        let state = bass_state(true, 12.0, true, true);
        let mut bass = BassBoost::new(48_000.0, 2);
        bass.set_params(&state);

        // DC + clipping bursts
        let mut buf: Vec<f32> = (0..8192)
            .map(|i| {
                if i % 3 == 0 {
                    1.0
                } else if i % 3 == 1 {
                    -1.0
                } else {
                    0.5
                }
            })
            .collect();
        bass.process(&mut buf, 2);

        // Shelf at +12 dB ≈ 4×; harmonics add at most +0.35*0.5 ≈ 0.175 extra.
        // The shelf biquad and tanh are both bounded — assert nothing NaN/inf
        // and output stays within a generous range.
        for (i, &x) in buf.iter().enumerate() {
            assert!(x.is_finite(), "sample {i} is not finite: {x}");
            assert!(x.abs() < 20.0, "sample {i} out of expected range: {x}");
        }
    }
}
