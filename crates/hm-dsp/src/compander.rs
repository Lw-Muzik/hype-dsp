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
//! reconstructs the input **exactly** (delayed by lookahead samples) — no comb
//! filtering, no RMS loss.
//!
//! **Lookahead:** each band holds per-channel ring buffers sized to
//! `LOOKAHEAD_MS` of delay. Gain is computed from the INCOMING sample but
//! applied to the DELAYED sample leaving the ring. All bands share the same
//! lookahead depth so the telescoping sum stays phase-aligned.
//!
//! Real-time safe: all band state is pre-allocated in `prepare`; `process` never
//! allocates/locks.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::CompanderState;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const BAND_COUNT: usize = 10;
const CROSSOVER_COUNT: usize = BAND_COUNT - 1; // 9
const CENTERS_HZ: [f32; BAND_COUNT] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// `20/ln(10)`. Only the exact [`lin_to_db`] reference oracle (test-only) needs
/// this; the real-time path uses [`lin_to_db_fast`], which never calls `ln`.
#[cfg(test)]
const LOG10_20: f32 = 8.685_889; // 20/ln(10)
const INV_LOG10_20: f32 = 0.115_129_255; // ln(10)/20
/// Fixed lookahead in milliseconds. Allocated in `prepare`; never re-allocated
/// during `process`. Adds ~3 ms of latency on top of any convolver delay.
const LOOKAHEAD_MS: f32 = 3.0;
/// Control-rate decimation of the dB→lin conversion (the per-band `exp`).
/// Everything upstream (envelope, static gain curve, ~2 ms de-click smoother)
/// stays per-frame with the original ballistics — none of it needs libm once
/// the envelope's `ln` is replaced by [`lin_to_db_fast`]. Per frame the linear
/// gain follows the smoothed dB gain by a second-order multiplicative update
/// (exact to ~1e-6 relative per frame, since the de-click smoother limits the
/// per-frame dB step to a few hundredths of a dB); the exact `exp` runs once
/// every this many frames purely to cancel the accumulated truncation drift.
const CTRL_INTERVAL: usize = 8;
/// dB per octave: `20·log10(2)`, converts `log2` to decibels.
const DB_PER_OCTAVE: f32 = 20.0 * std::f32::consts::LOG10_2;

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}
#[inline]
fn db_to_lin(db: f32) -> f32 {
    (db * INV_LOG10_20).exp()
}
/// Exact linear→dB via libm `ln`. Retained only as the reference oracle that
/// [`lin_to_db_fast`] is validated against in tests; the real-time path never
/// calls it.
#[cfg(test)]
#[inline]
fn lin_to_db(lin: f32) -> f32 {
    if lin < 1e-10 { -200.0 } else { lin.ln() * LOG10_20 }
}

/// Fast `lin_to_db`: exponent/mantissa split plus an atanh-series `log2` of
/// the mantissa, replacing the per-frame libm `ln` in the envelope follower.
///
/// With `m ∈ [1, 2)` and `t = (m−1)/(m+1) ∈ [0, ⅓]`,
/// `log2(m) = 2·atanh(t)/ln 2` and the odd series `t + t³/3 + t⁵/5 + t⁷/7`
/// truncates with error < 6e-6, so the result is within ~1e-4 dB of the exact
/// conversion (verified by `fast_db_conversion_is_accurate`) — four orders of
/// magnitude below audibility. Same `-200 dB` floor as `lin_to_db`.
#[inline]
fn lin_to_db_fast(lin: f32) -> f32 {
    if lin < 1e-10 {
        return -200.0;
    }
    // lin = m · 2^e with m ∈ [1, 2); 1e-10 > f32::MIN_POSITIVE so lin is
    // always a normal float here and the bit split is exact.
    let bits = lin.to_bits();
    let e = ((bits >> 23) as i32) - 127;
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let atanh = t * (1.0 + t2 * (1.0 / 3.0 + t2 * (0.2 + t2 * (1.0 / 7.0))));
    let log2_m = atanh * (2.0 / std::f32::consts::LN_2);
    (e as f32 + log2_m) * DB_PER_OCTAVE
}

// ---------------------------------------------------------------------------
// Per-band gain-reduction meter
// ---------------------------------------------------------------------------

/// Lock-free, real-time-safe GR meter: one f32-as-bits atomic per band.
///
/// Written (once per block) by the audio thread; read any time by the UI thread.
/// Uses `Relaxed` ordering — no synchronisation needed, just eventual visibility.
pub struct CompanderMeter {
    bands: [AtomicU32; BAND_COUNT],
}

impl Default for CompanderMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl CompanderMeter {
    pub fn new() -> Self {
        // SAFETY: 0.0_f32.to_bits() == 0; initialise all bands to 0 dB (no reduction).
        Self {
            bands: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
        }
    }

    /// Store gain-reduction `gr_db` (≤0) for `band` index. Audio-thread side.
    #[inline]
    pub fn store_band(&self, band: usize, gr_db: f32) {
        self.bands[band].store(gr_db.to_bits(), Ordering::Relaxed);
    }

    /// Read all 10 per-band GR values (dB, ≤0). UI-thread side.
    pub fn load(&self) -> [f32; BAND_COUNT] {
        std::array::from_fn(|i| f32::from_bits(self.bands[i].load(Ordering::Relaxed)))
    }
}

// ---------------------------------------------------------------------------
// Subtractive crossover (single channel)
// ---------------------------------------------------------------------------

/// One subtractive crossover for one channel: two cascaded Butterworth LP biquads.
/// The high portion is obtained by subtraction (`rest -= low`) — no HP biquads needed.
#[derive(Clone)]
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

// ---------------------------------------------------------------------------
// Per-band compressor
// ---------------------------------------------------------------------------

/// Per-band single-band compressor/expander (dB-domain), stereo-linked.
///
/// Gain ballistics: the envelope follower (level detection) uses `attack_coeff`
/// and `release_coeff`, setting the perceived attack/release speed. The gain
/// smoothing stage uses a fixed ~2 ms de-click coefficient, preventing zipper
/// noise without adding extra smoothing. Thus `attack_ms` and `release_ms`
/// govern compression/expansion response; note that for very fast attack_ms
/// (<~2ms) the fixed gain de-click floor limits how quickly gain changes apply.
///
/// Lookahead: incoming L/R are pushed into per-channel ring buffers
/// (`lookahead_samples` deep). Gain is derived from the INCOMING sample but
/// applied to the DELAYED sample popped from the ring, giving the compressor
/// look-ahead of `lookahead_samples` into the future.
///
/// Control-rate decimation: the envelope follower (via [`lin_to_db_fast`], no
/// libm call), static gain curve and de-click smoother all run per frame with
/// the original ballistics; the dB→linear conversion is tracked per frame by a
/// cheap second-order multiplicative update and re-synced by an exact `exp`
/// once every [`CTRL_INTERVAL`] frames.
struct BandCompressor {
    sample_rate: f32,
    env_db: f32,
    gain_smoothed_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    gain_declick_coeff: f32,
    // Control-rate decimation state for the dB→lin conversion.
    /// Frames until the next exact `db_to_lin` re-sync (1 ⇒ next frame syncs).
    ctrl_countdown: usize,
    /// Current per-frame linear gain (excluding makeup): tracks
    /// `db_to_lin(gain_smoothed_db)` multiplicatively between re-syncs.
    gain_lin: f32,
    // lookahead ring buffers (one per channel, allocated in prepare)
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    ring_pos: usize,
    lookahead_samples: usize,
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
        let ls = lookahead_samples(sample_rate);
        let mut s = Self {
            sample_rate,
            env_db: -96.0,
            gain_smoothed_db: 0.0,
            attack_coeff: 0.1,
            release_coeff: 0.001,
            gain_declick_coeff: 0.1,
            ctrl_countdown: 1,
            gain_lin: 1.0,
            ring_l: vec![0.0; ls.max(1)],
            ring_r: vec![0.0; ls.max(1)],
            ring_pos: 0,
            lookahead_samples: ls,
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

    fn prepare(&mut self, sample_rate: f32) {
        let ls = lookahead_samples(sample_rate);
        self.sample_rate = sample_rate;
        self.lookahead_samples = ls;
        // Resize rings (only allocates here, never in process).
        let cap = ls.max(1);
        self.ring_l = vec![0.0; cap];
        self.ring_r = vec![0.0; cap];
        self.ring_pos = 0;
    }

    fn recalc(&mut self, attack_ms: f32, release_ms: f32) {
        let a = (attack_ms * 0.001).max(0.001);
        let r = (release_ms * 0.001).max(0.001);
        self.attack_coeff = 1.0 - (-1.0 / (a * self.sample_rate)).exp();
        self.release_coeff = 1.0 - (-1.0 / (r * self.sample_rate)).exp();
        // Fixed ~2 ms gain de-click (fast, independent of attack_ms/release_ms).
        // This prevents zippered gain discontinuities while the envelope (via attack_coeff/
        // release_coeff) provides the perceptual attack/release time.
        self.gain_declick_coeff = 1.0 - (-1.0 / (0.002 * self.sample_rate)).exp();
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
        self.ctrl_countdown = 1; // first frame after reset re-syncs the gain
        self.gain_lin = 1.0;
        // Zero the rings to avoid stale audio bleeding across resets.
        for x in &mut self.ring_l { *x = 0.0; }
        for x in &mut self.ring_r { *x = 0.0; }
        self.ring_pos = 0;
    }

    /// Return the current smoothed gain in dB.
    #[inline]
    fn current_gain_db(&self) -> f32 {
        self.gain_smoothed_db
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

    /// Advance the gain path by one frame given the incoming (undelayed,
    /// future-looking) frame peak; returns the linear gain (including makeup)
    /// to apply to the delayed sample.
    ///
    /// The envelope, static gain curve and de-click smoother all run per frame
    /// with the original ballistics — the envelope's ripple on the rectified
    /// signal sets the steady-state gain, so it must not be decimated. Only the
    /// dB→lin conversion (the `exp`) is decimated.
    #[inline]
    fn advance_gain(&mut self, peak: f32) -> f32 {
        let peak_db = lin_to_db_fast(peak);
        if peak_db > self.env_db {
            self.env_db += self.attack_coeff * (peak_db - self.env_db);
        } else {
            self.env_db += self.release_coeff * (peak_db - self.env_db);
        }
        self.env_db = flush(self.env_db + 96.0) - 96.0;

        let gain_db = self.compute_gain(self.env_db);

        // Gain de-click: smooth using fixed fast ~2 ms coefficient (both directions).
        // The envelope (via attack/release_coeff above) governs the response speed;
        // this stage only de-zippers the gain output.
        let delta_db = self.gain_declick_coeff * (gain_db - self.gain_smoothed_db);
        self.gain_smoothed_db += delta_db;

        self.ctrl_countdown -= 1;
        if self.ctrl_countdown == 0 {
            // Exact re-sync: cancels the accumulated truncation drift of the
            // multiplicative tracking below (≲1e-5 relative per interval).
            self.ctrl_countdown = CTRL_INTERVAL;
            self.gain_lin = db_to_lin(self.gain_smoothed_db);
        } else {
            // Track the smoothed gain multiplicatively: gain·exp(a·Δ) with
            // exp(x) ≈ 1 + x + x²/2. The de-click smoother caps Δ at a few
            // hundredths of a dB per frame, so the truncation error is < 1e-6
            // relative — no libm call per frame.
            let x = INV_LOG10_20 * delta_db;
            self.gain_lin *= 1.0 + x * (1.0 + 0.5 * x);
        }
        self.gain_lin * self.makeup_lin
    }

    /// Process one stereo frame in place (peak-linked, with lookahead).
    ///
    /// Gain is computed from the INCOMING `(l, r)` sample (future-looking), but
    /// applied to the DELAYED sample popped from the ring buffer. When
    /// `lookahead_samples == 0` the ring degrades to a 1-sample dummy and the
    /// output is the incoming sample with current gain — no meaningful delay.
    #[inline]
    fn process_frame(&mut self, l: &mut f32, r: &mut f32) {
        // 1. Envelope (per-frame) + decimated gain from incoming (undelayed) level.
        let g = self.advance_gain(l.abs().max(r.abs()));

        // 2. Lookahead ring: push incoming, pop delayed.
        if self.lookahead_samples == 0 {
            // No lookahead: apply gain to current sample.
            *l *= g;
            *r *= g;
        } else {
            // ring_pos is always in [0, ring_l.len()) — the increment wraps.
            let pos = self.ring_pos;
            let delayed_l = self.ring_l[pos];
            let delayed_r = self.ring_r[pos];
            self.ring_l[pos] = *l;
            self.ring_r[pos] = *r;
            self.ring_pos += 1;
            if self.ring_pos == self.ring_l.len() {
                self.ring_pos = 0;
            }
            *l = delayed_l * g;
            *r = delayed_r * g;
        }
    }

    /// Mono variant of [`process_frame`](Self::process_frame): with `l == r`
    /// the stereo-linked gain is identical, so the right ring/crossover work is
    /// pure duplication and is skipped entirely.
    #[inline]
    fn process_frame_mono(&mut self, l: &mut f32) {
        let g = self.advance_gain(l.abs());
        if self.lookahead_samples == 0 {
            *l *= g;
        } else {
            let pos = self.ring_pos;
            let delayed_l = self.ring_l[pos];
            self.ring_l[pos] = *l;
            self.ring_pos += 1;
            if self.ring_pos == self.ring_l.len() {
                self.ring_pos = 0;
            }
            *l = delayed_l * g;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lookahead_samples(sample_rate: f32) -> usize {
    (LOOKAHEAD_MS * 0.001 * sample_rate).round() as usize
}

// ---------------------------------------------------------------------------
// Compander
// ---------------------------------------------------------------------------

/// The 10-band compander stage.
pub struct Compander {
    sample_rate: f32,
    enabled: bool,
    crossovers_l: Vec<LrChannel>, // len CROSSOVER_COUNT
    crossovers_r: Vec<LrChannel>,
    bands: Vec<BandCompressor>, // len BAND_COUNT
    /// Per-band gain-reduction meter (written once per block).
    meter: Arc<CompanderMeter>,
    /// Change-guard: last applied compander params so `set_params` can skip the
    /// ≈20 `exp()` coefficient recomputes when nothing changed. Cleared by
    /// `prepare` (sample rate change invalidates rate-dependent coefficients).
    cached: Option<CompanderState>,
}

impl Compander {
    /// Create a compander wired to a caller-owned meter.
    pub fn with_meter(sample_rate: f32, _channels: usize, meter: Arc<CompanderMeter>) -> Self {
        let mut s = Self {
            sample_rate,
            enabled: false,
            crossovers_l: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            crossovers_r: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            bands: (0..BAND_COUNT).map(|_| BandCompressor::new(sample_rate)).collect(),
            meter,
            cached: None,
        };
        s.reconfigure();
        s
    }

    /// Create a compander with a private (throwaway) meter.
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self::with_meter(sample_rate, channels, Arc::new(CompanderMeter::default()))
    }

    /// Access the shared meter handle (clone the Arc to share with other threads).
    pub fn meter(&self) -> Arc<CompanderMeter> {
        Arc::clone(&self.meter)
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
            b.prepare(sample_rate);
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
            // Write zeros to the meter so the UI shows no reduction when disabled.
            for i in 0..BAND_COUNT {
                self.meter.store_band(i, 0.0);
            }
            return;
        }
        let frames = buffer.len() / channels;
        let stereo = channels >= 2;

        // Track minimum (most-negative) gain_smoothed_db per band across the block.
        // Initialize to 0.0 (no reduction); will be updated to the min seen.
        let mut min_gr_per_band = [0.0_f32; BAND_COUNT];

        if stereo {
            for f in 0..frames {
                let base = f * channels;
                let in_l = buffer[base];
                let in_r = buffer[base + 1];
                // Subtractive (telescoping) crossover:
                //   rest -= low_i  (exact subtraction — no HP biquads)
                //   Σ low_i + rest_9 = input by construction → flat = transparent.
                let mut rest_l = in_l;
                let mut rest_r = in_r;
                let mut sum_l = 0.0_f32;
                let mut sum_r = 0.0_f32;
                for (i, min_gr) in min_gr_per_band[..CROSSOVER_COUNT].iter_mut().enumerate() {
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
                    // Track minimum gain for this band.
                    let gr = self.bands[i].current_gain_db();
                    *min_gr = min_gr.min(gr);
                }
                // Final (highest) band = whatever remains after all LP subtractions.
                let (mut bl, mut br) = (rest_l, rest_r);
                self.bands[BAND_COUNT - 1].process_frame(&mut bl, &mut br);
                sum_l += bl;
                sum_r += br;
                // Track minimum gain for the final band.
                let gr = self.bands[BAND_COUNT - 1].current_gain_db();
                min_gr_per_band[BAND_COUNT - 1] = min_gr_per_band[BAND_COUNT - 1].min(gr);

                buffer[base] = flush(sum_l).clamp(-4.0, 4.0);
                buffer[base + 1] = flush(sum_r).clamp(-4.0, 4.0);
            }
        } else {
            // Mono: with `in_r == in_l` the right crossover tree and ring
            // duplicate the left bit-for-bit and the stereo-linked gain is the
            // same, so the whole R half is skipped (halves the work).
            for sample in buffer.iter_mut() {
                let mut rest_l = *sample;
                let mut sum_l = 0.0_f32;
                for (i, min_gr) in min_gr_per_band[..CROSSOVER_COUNT].iter_mut().enumerate() {
                    let low_l = self.crossovers_l[i].lowpass(rest_l);
                    rest_l -= low_l;
                    let mut bl = low_l;
                    self.bands[i].process_frame_mono(&mut bl);
                    sum_l += bl;
                    *min_gr = min_gr.min(self.bands[i].current_gain_db());
                }
                let mut bl = rest_l;
                self.bands[BAND_COUNT - 1].process_frame_mono(&mut bl);
                sum_l += bl;
                min_gr_per_band[BAND_COUNT - 1] =
                    min_gr_per_band[BAND_COUNT - 1].min(self.bands[BAND_COUNT - 1].current_gain_db());

                *sample = flush(sum_l).clamp(-4.0, 4.0);
            }
        }

        // Publish GR once per block. Write the peak (minimum, most-negative) gain
        // reduction in dB (≤0) seen across all frames in this block, so transient
        // pumping is visible. Cheap atomic store per band.
        for (i, &min_gr) in min_gr_per_band.iter().enumerate() {
            self.meter.store_band(i, min_gr);
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
        EngineState { compander: c, ..Default::default() }
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

    // -----------------------------------------------------------------------
    // Existing tests (must still pass)
    // -----------------------------------------------------------------------

    #[test]
    fn disabled_is_identity() {
        let mut c = Compander::new(48_000.0, 2);
        // Default state: enabled = false
        c.set_params(&EngineState::default());
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let buf = input.clone();
        // interleave as stereo
        let mut stereo: Vec<f32> = buf.iter().flat_map(|&x| [x, x]).collect();
        let orig = stereo.clone();
        c.process(&mut stereo, 2);
        assert_eq!(stereo, orig, "disabled compander must be bit-exact identity");
        // also mono
        let mut mono = input.clone();
        let orig2 = mono.clone();
        c.process(&mut mono, 1);
        assert_eq!(mono, orig2, "disabled compander must be bit-exact identity (mono)");
    }

    #[test]
    fn flat_compander_reconstructs_input() {
        // With lookahead the output is the input delayed by L = lookahead_samples.
        // Verify: out[L..] ≈ in[0..len-L] with RMS error < 2%.
        let sr = 48_000.0_f32;
        let l_samples = lookahead_samples(sr); // typically 144 at 48 kHz

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

        // Prime a few blocks so filters and envelopes settle (also pre-fills rings).
        let prime_frames = 4096_usize;
        let prime_samples = prime_frames * 2;
        let mut prime_buf = signal[..prime_samples].to_vec();
        c.process(&mut prime_buf, 2);

        // Process the rest.
        let mut out_buf = signal[prime_samples..].to_vec();
        let in_slice = &signal[prime_samples..];
        c.process(&mut out_buf, 2);

        // With lookahead: out[L*2..] (interleaved stereo) ≈ in[0..len-L*2].
        // L stereo-frames = L*2 samples in the interleaved buffer.
        let l_stereo = l_samples * 2; // samples to skip at the start of output
        if l_stereo >= out_buf.len() {
            // Edge case: lookahead >= block — skip reconstruction check.
            return;
        }

        let out_shifted = &out_buf[l_stereo..];
        let in_ref = &in_slice[..out_shifted.len()];

        let err_rms = {
            let sum_sq: f32 =
                out_shifted.iter().zip(in_ref.iter()).map(|(o, i)| (o - i) * (o - i)).sum();
            (sum_sq / out_shifted.len() as f32).sqrt()
        };
        let in_rms = rms(in_ref);
        let rel_err = err_rms / in_rms;

        assert!(
            rel_err < 0.02,
            "subtractive crossover must reconstruct input (shifted by {} lookahead frames) \
             with <2% RMS error, got {:.4}% (err_rms={err_rms:.6}, in_rms={in_rms:.6})",
            l_samples,
            rel_err * 100.0
        );
    }

    #[test]
    fn fast_attack_reduces_faster_than_slow() {
        // A compander with 1 ms attack should reach its compressed level in fewer
        // samples than one with 100 ms attack on the same loud step input.
        let sr = 48_000.0_f32;
        let loud_params = |attack_ms: f32| {
            compander_params(CompanderState {
                enabled: true,
                ratio: 10.0,
                threshold_db: -30.0,
                knee_db: 0.0,
                gate_db: -90.0,
                attack_ms,
                release_ms: 200.0,
                makeup_db: 0.0,
                expander_ratio: 1.0,
            })
        };

        let frames = 4096_usize;
        // Loud sustained sine well above threshold (amplitude ≈ 1.0 → ~0 dBFS)
        let signal: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin() * 0.95;
                [s, s]
            })
            .collect();

        // Fast-attack compander
        let mut fast = Compander::new(sr, 2);
        fast.set_params(&loud_params(1.0));
        let mut fast_buf = signal.clone();
        fast.process(&mut fast_buf, 2);

        // Slow-attack compander
        let mut slow = Compander::new(sr, 2);
        slow.set_params(&loud_params(100.0));
        let mut slow_buf = signal.clone();
        slow.process(&mut slow_buf, 2);

        // Find the frame at which each compander SUSTAINS below 50% of input peak.
        // Skip the initial lookahead-zeros (rings start empty → output is 0).
        let in_peak = 0.95_f32;
        let target = in_peak * 0.5;
        let ls = lookahead_samples(sr); // skip zeroed lookahead prefix

        // After the lookahead prefix the signal is the delayed (uncompressed) input; the
        // compander starts chasing the gain. Find the first frame (past the prefix) where
        // the output first drops and then STAYS below target for the rest of the buffer
        // — that index measures when the gain has fully settled.
        let settled_idx = |buf: &[f32]| -> usize {
            // Scan stereo frames past the lookahead prefix.
            let start = ls.min(buf.len() / 2);
            for i in start..(buf.len() / 2) {
                let sample = buf[i * 2].abs();
                // Check that from frame i onward the signal is suppressed.
                if sample < target {
                    // Verify it stays suppressed for the next ~50 frames.
                    let check_end = (i + 50).min(buf.len() / 2);
                    let stays = (i..check_end).all(|j| buf[j * 2].abs() < target);
                    if stays {
                        return i;
                    }
                }
            }
            buf.len() / 2 // did not settle
        };

        let fast_idx = settled_idx(&fast_buf);
        let slow_idx = settled_idx(&slow_buf);

        assert!(
            fast_idx < slow_idx,
            "fast attack (1 ms) should reach compression target in fewer frames \
             than slow attack (100 ms): fast={fast_idx} slow={slow_idx}"
        );
    }

    #[test]
    fn meter_reports_reduction_under_compression() {
        let sr = 48_000.0_f32;

        // Heavy compression: low threshold, high ratio → large GR on a loud signal.
        let heavy_params = compander_params(CompanderState {
            enabled: true,
            ratio: 20.0,
            threshold_db: -40.0,
            knee_db: 0.0,
            gate_db: -90.0,
            attack_ms: 1.0,
            release_ms: 50.0,
            makeup_db: 0.0,
            expander_ratio: 1.0,
        });

        let meter = Arc::new(CompanderMeter::new());
        let mut c = Compander::with_meter(sr, 2, Arc::clone(&meter));
        c.set_params(&heavy_params);

        // Loud signal well above threshold
        let frames = 4096_usize;
        let signal: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin() * 0.9;
                [s, s]
            })
            .collect();

        // Prime to settle
        let mut prime = signal.clone();
        c.process(&mut prime, 2);
        // Process again — meter should now reflect steady-state GR.
        let mut buf = signal.clone();
        c.process(&mut buf, 2);

        let gr = meter.load();
        let any_reduction = gr.iter().any(|&g| g < -1.0); // at least 1 dB reduction
        assert!(
            any_reduction,
            "meter must report GR < −1 dB on at least one band under heavy compression; got {gr:?}"
        );

        // Disabled compander → meter must report ~0.
        let mut c_off = Compander::with_meter(sr, 2, Arc::clone(&meter));
        c_off.set_params(&EngineState::default()); // enabled=false
        let mut buf2 = signal.clone();
        c_off.process(&mut buf2, 2);
        let gr_off = meter.load();
        assert!(
            gr_off.iter().all(|&g| g.abs() < 0.01),
            "disabled compander must leave meter at ~0 dB; got {gr_off:?}"
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
        let mut buf: Vec<f32> = (0..8192).flat_map(|_| [1.0_f32, -1.0_f32]).collect();
        c.process(&mut buf, 2);
        assert!(
            buf.iter().all(|&x| x.abs() <= 4.0),
            "compander output must stay within ±4.0"
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

    /// Reference: the original PER-FRAME gain path (per-frame coefficients,
    /// envelope from the instantaneous peak, dB de-click, dB→lin every frame),
    /// kept verbatim so the CTRL_INTERVAL-decimated path can be compared to it.
    struct RefBand {
        env_db: f32,
        gain_smoothed_db: f32,
        attack_coeff: f32,
        release_coeff: f32,
        gain_declick_coeff: f32,
        ring: Vec<f32>,
        ring_pos: usize,
    }

    impl RefBand {
        fn new(sr: f32, attack_ms: f32, release_ms: f32) -> Self {
            let a = (attack_ms * 0.001).max(0.001);
            let r = (release_ms * 0.001).max(0.001);
            Self {
                env_db: -96.0,
                gain_smoothed_db: 0.0,
                attack_coeff: 1.0 - (-1.0 / (a * sr)).exp(),
                release_coeff: 1.0 - (-1.0 / (r * sr)).exp(),
                gain_declick_coeff: 1.0 - (-1.0 / (0.002 * sr)).exp(),
                ring: vec![0.0; lookahead_samples(sr).max(1)],
                ring_pos: 0,
            }
        }

        /// One mono frame, original per-frame math. `curve` supplies the
        /// (stateless) static gain curve so it is shared with the real band.
        fn process(&mut self, x: f32, curve: &BandCompressor) -> f32 {
            let peak_db = lin_to_db(x.abs());
            if peak_db > self.env_db {
                self.env_db += self.attack_coeff * (peak_db - self.env_db);
            } else {
                self.env_db += self.release_coeff * (peak_db - self.env_db);
            }
            self.env_db = flush(self.env_db + 96.0) - 96.0;
            let gain_db = curve.compute_gain(self.env_db);
            self.gain_smoothed_db +=
                self.gain_declick_coeff * (gain_db - self.gain_smoothed_db);
            let g = db_to_lin(self.gain_smoothed_db) * curve.makeup_lin;
            let pos = self.ring_pos;
            let delayed = self.ring[pos];
            self.ring[pos] = x;
            self.ring_pos = (self.ring_pos + 1) % self.ring.len();
            delayed * g
        }
    }

    /// The control-rate-decimated band must stay within a tight tolerance of
    /// the per-frame reference on a burst (attack, sustain, and release).
    #[test]
    fn decimated_band_matches_per_frame_reference() {
        let sr = 48_000.0_f32;
        let params = compander_params(CompanderState {
            enabled: true,
            ratio: 4.0,
            threshold_db: -30.0,
            knee_db: 0.0,
            gate_db: -90.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            makeup_db: 0.0,
            expander_ratio: 1.0,
        });

        // Loud 1 kHz burst, then near-silence: exercises attack, steady-state
        // gain reduction, and release.
        let burst = 7_200_usize; // 150 ms
        let total = 14_400_usize; // + 150 ms tail
        let signal: Vec<f32> = (0..total)
            .map(|i| {
                let amp = if i < burst { 0.9 } else { 0.02 };
                (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / sr).sin() * amp
            })
            .collect();

        let mut band = BandCompressor::new(sr);
        band.set_params(&params);
        let mut reference = RefBand::new(sr, 5.0, 50.0);

        let mut err_sq = 0.0_f64;
        let mut ref_sq = 0.0_f64;
        let mut max_diff = 0.0_f32;
        for &x in &signal {
            let mut got = x;
            band.process_frame_mono(&mut got);
            let want = reference.process(x, &band);
            let d = (got - want).abs();
            max_diff = max_diff.max(d);
            err_sq += (d as f64) * (d as f64);
            ref_sq += (want as f64) * (want as f64);
        }
        let rel_rms = (err_sq / ref_sq.max(1e-30)).sqrt();
        assert!(
            rel_rms < 0.001,
            "decimated gain path must track the per-frame reference: \
             rel RMS error {:.4}% (max sample diff {max_diff:.6})",
            rel_rms * 100.0
        );
        assert!(
            max_diff < 0.005,
            "decimated gain path max sample deviation too large: {max_diff:.6}"
        );
    }

    /// The polynomial dB conversion must agree with the exact libm one to well
    /// under a thousandth of a dB across the whole audio range.
    #[test]
    fn fast_db_conversion_is_accurate() {
        // Dense sweep across 10 decades (just above the -200 dB floor to +12 dB).
        let mut lin = 2e-10_f32;
        let mut max_err = 0.0_f32;
        while lin < 4.0 {
            let exact = lin_to_db(lin);
            let fast = lin_to_db_fast(lin);
            max_err = max_err.max((exact - fast).abs());
            lin *= 1.001;
        }
        assert!(
            max_err < 1e-3,
            "lin_to_db_fast must be within 0.001 dB of libm, got {max_err}"
        );
        // Floor behaviour must match exactly.
        assert_eq!(lin_to_db_fast(0.0), -200.0);
        assert_eq!(lin_to_db_fast(9e-11), -200.0);
    }

    /// Mono processing (which skips the duplicated right-channel work) must
    /// match the left channel of the equivalent dual-mono stereo run exactly.
    #[test]
    fn mono_matches_dual_mono_stereo() {
        let sr = 48_000.0_f32;
        let params = compander_params(CompanderState {
            enabled: true,
            ratio: 4.0,
            threshold_db: -24.0,
            knee_db: 6.0,
            gate_db: -70.0,
            attack_ms: 10.0,
            release_ms: 80.0,
            makeup_db: 3.0,
            expander_ratio: 2.0,
        });
        let mono: Vec<f32> = (0..8_192)
            .map(|i| (2.0 * std::f32::consts::PI * 500.0 * i as f32 / sr).sin() * 0.8)
            .collect();

        let mut c_mono = Compander::new(sr, 1);
        c_mono.set_params(&params);
        let mut mono_buf = mono.clone();
        c_mono.process(&mut mono_buf, 1);

        let mut c_stereo = Compander::new(sr, 2);
        c_stereo.set_params(&params);
        let mut stereo_buf: Vec<f32> = mono.iter().flat_map(|&x| [x, x]).collect();
        c_stereo.process(&mut stereo_buf, 2);

        for (f, &m) in mono_buf.iter().enumerate() {
            let l = stereo_buf[f * 2];
            assert!(
                m.to_bits() == l.to_bits(),
                "frame {f}: mono path {m} != dual-mono stereo left {l}"
            );
        }
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
