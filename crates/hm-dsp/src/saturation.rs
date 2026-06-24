//! Tube-style saturation stage — 4× oversampled asymmetric waveshaper.
//!
//! # Signal path (per channel)
//!
//! ```text
//!  in ──┬──────────────────────────────── dry-delay(32) ──┐
//!       │                                                  │
//!       └── upsample(4×) → shape() → downsample(4×) → DC-block ──┤
//!                                                          │
//!                       out = dry*(1-mix) + wet*mix*makeup ┘
//! ```
//!
//! # Waveshaper
//!
//! ```text
//! shape(x, drive, bias) = tanh(drive*(x + bias)) − tanh(drive*bias)
//! ```
//!
//! `drive` is mapped from the 0..1 UI parameter to the range 1..10.
//! The fixed `bias = 0.2` creates asymmetry, generating 2nd-order (even)
//! harmonics characteristic of triode tube saturation.  The
//! `tanh(drive*bias)` subtraction removes the static DC offset that
//! asymmetric bias would otherwise produce.
//!
//! # Auto makeup
//!
//! The shaper compresses peaks at high drive; to keep the overall loudness
//! roughly constant we apply:
//!
//! ```text
//! makeup = 1.0 / shape(1.0, drive_mapped, BIAS)
//! ```
//!
//! This is the reciprocal of the shaper's response to a unit-amplitude
//! input, computed once in `set_params` (no per-sample cost).  At drive=0
//! (drive_mapped≈1, tanh≈identity) makeup≈1; at drive=1 (drive_mapped=10,
//! tanh strongly limiting) makeup compensates the reduced peak.
//!
//! # DC blocker
//!
//! A one-pole high-pass runs at base rate after downsampling:
//! `y[n] = x[n] − x_prev + R*y_prev`,  R = 0.999 (≈7 Hz corner @48 kHz).
//! This removes any residual low-frequency DC that survives the waveshaper's
//! static subtraction. State is denormal-flushed each sample.
//!
//! # Dry-delay alignment
//!
//! The wet path introduces `Oversampler4x::latency_samples()` = 32 base-rate
//! samples of group delay.  The dry path is run through a circular delay
//! line of the same length so the two paths are time-aligned when mixing.
//!
//! # RT safety
//!
//! `process()` never allocates.  All buffers (`up_scratch`, `dn_scratch`,
//! dry delay lines) are pre-sized in `prepare`.

use crate::oversample::Oversampler4x;
use crate::{AudioProcessor, ProcessorParams};

/// Fixed bias for asymmetric tube character (generates 2nd-order harmonics).
const BIAS: f32 = 0.2;

/// DC blocker pole radius (≈7 Hz corner at 48 kHz).
const DC_R: f32 = 0.999;

/// Maximum block size for pre-sizing scratch buffers (prevents RT-thread allocation).
const MAX_BLOCK_FRAMES: usize = 4096;

/// Flush near-zero values to avoid denormal CPU penalties on IIR state.
#[inline(always)]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

/// Asymmetric tube waveshaper with static DC offset removed.
///
/// `drive` is the *mapped* drive value (1..10), **not** the raw 0..1 param.
#[inline(always)]
fn shape(x: f32, drive: f32) -> f32 {
    (drive * (x + BIAS)).tanh() - (drive * BIAS).tanh()
}

/// One-channel DC blocker.
struct DcBlocker {
    x_prev: f32,
    y_prev: f32,
}

impl DcBlocker {
    fn new() -> Self {
        Self { x_prev: 0.0, y_prev: 0.0 }
    }

    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        let y = flush(x - self.x_prev + DC_R * self.y_prev);
        self.x_prev = x;
        self.y_prev = y;
        y
    }

    fn reset(&mut self) {
        self.x_prev = 0.0;
        self.y_prev = 0.0;
    }
}

/// One-channel integer-sample delay line (circular buffer).
struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0f32; len.max(1)],
            pos: 0,
        }
    }

    /// Resize the delay line (only from `prepare`; not called on RT thread).
    fn resize(&mut self, len: usize) {
        self.buf = vec![0.0f32; len.max(1)];
        self.pos = 0;
    }

    /// Push `x` and return the sample that comes out `len` samples later.
    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        let n = self.buf.len();
        let out = self.buf[self.pos];
        self.buf[self.pos] = x;
        self.pos = if self.pos + 1 == n { 0 } else { self.pos + 1 };
        out
    }

}

/// Tube saturation stage: 4× oversampled asymmetric waveshaper, DC blocker,
/// dry-delay alignment, auto makeup.
pub struct Saturation {
    // ── per-channel DSP state ──────────────────────────────────────────────
    /// One oversampler per channel (at most 2: L/R).
    oversamplers: [Oversampler4x; 2],
    dc_blockers: [DcBlocker; 2],
    dry_delays: [DelayLine; 2],

    // ── pre-sized scratch buffers (RT-safe: no alloc in process) ──────────
    /// Scratch for the 4× upsampled signal (length = max_frames * 4).
    up_scratch: Vec<f32>,
    /// Scratch for the downsampled wet signal (length = max_frames).
    dn_scratch: Vec<f32>,

    // ── cached params (change-guarded) ────────────────────────────────────
    enabled: bool,
    /// Raw drive parameter (0..1), used only for change detection.
    drive_raw: f32,
    /// Raw mix parameter (0..1), used only for change detection.
    mix_raw: f32,
    /// Mapped drive (1..10) used in the waveshaper.
    drive_mapped: f32,
    /// Mix (0..1); `1 - mix` is the dry fraction.
    mix: f32,
    /// Auto-makeup gain (recomputed from drive_mapped in set_params).
    makeup: f32,
}

impl Saturation {
    /// Create a new `Saturation` stage.
    ///
    /// `channels` is the maximum expected channel count (1 or 2).
    /// Call [`prepare`](AudioProcessor::prepare) whenever the stream format is
    /// known; this pre-sizes the scratch buffers.
    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let (drive_mapped, makeup) = Self::compute_drive_makeup(0.3);
        Self {
            oversamplers: [Oversampler4x::new(sample_rate), Oversampler4x::new(sample_rate)],
            dc_blockers: [DcBlocker::new(), DcBlocker::new()],
            dry_delays: [
                DelayLine::new(Oversampler4x::new(sample_rate).latency_samples()),
                DelayLine::new(Oversampler4x::new(sample_rate).latency_samples()),
            ],
            up_scratch: Vec::new(),
            dn_scratch: Vec::new(),
            enabled: false,
            drive_raw: 0.3,
            mix_raw: 1.0,
            drive_mapped,
            mix: 1.0,
            makeup,
        }
    }

    /// Compute the mapped drive value and auto-makeup gain from the raw 0..1 param.
    ///
    /// `drive_mapped = 1.0 + raw * 9.0`  → range [1, 10].
    ///
    /// **Makeup formula**: `makeup = ref_rms / shaped_rms`, where
    /// - `ref_rms = 0.5 / √2 ≈ 0.354` — RMS of a 0.5-amplitude reference sine.
    /// - `shaped_rms` — RMS of that sine after running through the waveshaper.
    ///
    /// This is computed via a short numerical integral over one full sine cycle
    /// (256 steps). Because the asymmetric shaper can produce RMS *greater*
    /// than the input (the negative half extends far with positive bias), we
    /// must use the full RMS of the shaped wave, not just a peak estimate.
    /// Recomputed only in `set_params` (change-guarded), never in `process`.
    fn compute_drive_makeup(drive_raw: f32) -> (f32, f32) {
        use std::f32::consts::PI;
        const REF_AMP: f32 = 0.5;
        const STEPS: usize = 256; // one cycle, enough precision

        let drive_mapped = 1.0 + drive_raw.clamp(0.0, 1.0) * 9.0;

        // Numerically integrate RMS of shaped sine over one period.
        let rms_sq: f32 = (0..STEPS)
            .map(|i| {
                let x = REF_AMP * (2.0 * PI * i as f32 / STEPS as f32).sin();
                let s = shape(x, drive_mapped);
                s * s
            })
            .sum::<f32>()
            / STEPS as f32;
        let shaped_rms = rms_sq.sqrt().max(1e-6);

        // Reference input RMS for a sine of amplitude REF_AMP = REF_AMP / √2.
        let ref_rms = REF_AMP / 2.0_f32.sqrt();

        let makeup = ref_rms / shaped_rms;
        (drive_mapped, makeup)
    }

    /// Process a single channel from `frames` frames of interleaved `buffer`,
    /// reading/writing stride `channels` starting at `ch`.
    fn process_channel(
        &mut self,
        buffer: &mut [f32],
        channels: usize,
        ch: usize,
        frames: usize,
    ) {
        let drive = self.drive_mapped;
        let mix = self.mix;
        let makeup = self.makeup;
        let up_len = frames * 4;

        // Extract channel signal into up_scratch (reuse as mono input temp).
        // We need a contiguous slice for the oversampler; use dn_scratch as
        // the mono input temp (it's frames-long) and up_scratch for 4× output.
        for f in 0..frames {
            self.dn_scratch[f] = buffer[f * channels + ch];
        }

        // ── wet path ─────────────────────────────────────────────────────
        self.oversamplers[ch].upsample(&self.dn_scratch[..frames], &mut self.up_scratch[..up_len]);

        for v in &mut self.up_scratch[..up_len] {
            *v = shape(*v, drive);
        }

        // Temp store for downsampled wet: reuse a region of dn_scratch offset
        // by 0 (we read the dry from buffer again below; dn_scratch is OK to
        // overwrite now).
        self.oversamplers[ch].downsample(&self.up_scratch[..up_len], &mut self.dn_scratch[..frames]);

        // DC-block the wet signal.
        for f in 0..frames {
            self.dn_scratch[f] = self.dc_blockers[ch].process(self.dn_scratch[f]);
        }

        // ── dry path + mix ────────────────────────────────────────────────
        for f in 0..frames {
            let dry_in = buffer[f * channels + ch];
            let dry_delayed = self.dry_delays[ch].process(dry_in);
            let wet = self.dn_scratch[f];
            let mixed = dry_delayed * (1.0 - mix) + wet * mix * makeup;
            buffer[f * channels + ch] = mixed.clamp(-4.0, 4.0);
        }
    }
}

impl AudioProcessor for Saturation {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        // Rebuild oversamplers for the new rate.
        self.oversamplers = [
            Oversampler4x::new(sample_rate),
            Oversampler4x::new(sample_rate),
        ];
        let lat = self.oversamplers[0].latency_samples();
        self.dry_delays[0].resize(lat);
        self.dry_delays[1].resize(lat);
        self.dc_blockers[0].reset();
        self.dc_blockers[1].reset();

        // Pre-size scratch buffers to avoid RT-thread allocation in process().
        // up_scratch is 4× upsampled, dn_scratch is base rate.
        self.up_scratch.resize(MAX_BLOCK_FRAMES * 4, 0.0);
        self.dn_scratch.resize(MAX_BLOCK_FRAMES, 0.0);
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 || buffer.is_empty() {
            return; // bit-exact identity when disabled
        }

        let frames = buffer.len() / channels;
        let up_len = frames * 4;

        // Grow scratch off the RT path when block size increases; this is
        // typically called once at prepare time, not per-block.
        if self.up_scratch.len() < up_len {
            self.up_scratch.resize(up_len, 0.0);
        }
        if self.dn_scratch.len() < frames {
            self.dn_scratch.resize(frames, 0.0);
        }

        // Channel 0 (L or mono).
        self.process_channel(buffer, channels, 0, frames);
        // Channel 1 (R); if mono, mirror L processing with ch=0 oversampler
        // (channels == 1 means the outer loop above already handled it).
        if channels >= 2 {
            self.process_channel(buffer, channels, 1, frames);
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        let s = &params.saturation;
        self.enabled = s.enabled;

        let drive_changed = (self.drive_raw - s.drive).abs() > f32::EPSILON;
        let mix_changed = (self.mix_raw - s.mix).abs() > f32::EPSILON;

        if drive_changed {
            self.drive_raw = s.drive;
            let (dm, mk) = Self::compute_drive_makeup(s.drive);
            self.drive_mapped = dm;
            self.makeup = mk;
        }
        if mix_changed {
            self.mix_raw = s.mix;
            self.mix = s.mix.clamp(0.0, 1.0);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, SaturationState};
    use realfft::RealFftPlanner;

    const SR: f32 = 48_000.0;

    fn make_params(enabled: bool, drive: f32, mix: f32) -> EngineState {
        EngineState {
            saturation: SaturationState { enabled, drive, mix },
            ..Default::default()
        }
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|&v| v * v).sum::<f32>() / buf.len() as f32).sqrt()
    }

    // ── disabled_is_identity ─────────────────────────────────────────────────

    #[test]
    fn disabled_is_identity() {
        let mut sat = Saturation::new(SR, 2);
        sat.set_params(&make_params(false, 0.5, 1.0));
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin() * 0.8).collect();
        let stereo: Vec<f32> = input.iter().flat_map(|&v| [v, v]).collect();
        let mut buf = stereo.clone();
        sat.process(&mut buf, 2);
        assert_eq!(buf, stereo, "disabled must be bit-exact identity");
    }

    // ── mix_zero_is_dry_delayed ──────────────────────────────────────────────

    #[test]
    fn mix_zero_is_dry_delayed() {
        let lat = Oversampler4x::new(SR).latency_samples();
        let frames = 512;

        let mut sat = Saturation::new(SR, 2);
        sat.set_params(&make_params(true, 0.5, 0.0)); // mix=0 → pure dry path

        // Mono sine, stereo interleaved.
        let input: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * 440.0 / SR * i as f32).sin() * 0.5;
                [s, s]
            })
            .collect();
        let mut buf = input.clone();
        sat.process(&mut buf, 2);

        // After the dry delay, each output sample should equal input[f - lat].
        // The first `lat` output samples come from the delay line's initial zeros.
        for f in lat..frames {
            // The delay line outputs input[f - lat] at frame f.
            // Since we feed input[f] at frame f, the output at frame f is input[f-lat].
            let got_l = buf[f * 2];
            let expected_shifted = input[(f - lat) * 2];
            assert!(
                (got_l - expected_shifted).abs() < 1e-5,
                "frame {f}: expected dry-delayed sample {expected_shifted:.6}, got {got_l:.6} (lat={lat})"
            );
        }
    }

    // ── produces_even_harmonic ───────────────────────────────────────────────

    #[test]
    fn produces_even_harmonic() {
        const N: usize = 8192;
        const FREQ: f32 = 1_000.0;

        let run = |enabled: bool, drive: f32| -> Vec<f32> {
            let mut sat = Saturation::new(SR, 1);
            sat.set_params(&make_params(enabled, drive, 1.0));
            // Mono buffer
            let mut buf: Vec<f32> = (0..N)
                .map(|i| (2.0 * std::f32::consts::PI * FREQ / SR * i as f32).sin() * 0.8)
                .collect();
            sat.process(&mut buf, 1);
            buf
        };

        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = lat * 2 + 64; // discard transient

        // High-drive enabled output
        let out_on = run(true, 0.9);
        // Disabled (identity) output — should have no 2nd harmonic
        let out_off = run(false, 0.9);

        let seg_len = N - skip;

        let fft_mag = |sig: &[f32]| -> f32 {
            let seg = &sig[skip..skip + seg_len];
            let n = seg.len();
            let mut buf = seg.to_vec();
            let mut planner = RealFftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(n);
            let mut spec = fft.make_output_vec();
            fft.process(&mut buf, &mut spec).unwrap();
            let bin_2f = (2.0 * FREQ / SR * seg_len as f32).round() as usize;
            spec[bin_2f.min(spec.len() - 1)].norm() / n as f32
        };

        let mag_on = fft_mag(&out_on);
        let mag_off = fft_mag(&out_off);

        assert!(
            mag_on > mag_off * 5.0,
            "2nd harmonic should be clearly higher with saturation on: on={mag_on:.6}, off={mag_off:.6}"
        );
    }

    // ── makeup_keeps_level_roughly_stable ────────────────────────────────────

    #[test]
    fn makeup_keeps_level_roughly_stable() {
        // Use 2 s of audio so the DC blocker (R=0.999, τ≈1000 samples) has time
        // to settle before we measure RMS.  The DC blocker's initial transient
        // after the asymmetric-shaper warmup can take thousands of samples to
        // decay, so we skip 6000 samples and measure the last ~90 000 samples.
        const N: usize = 96_000; // 2 s @48k
        const FREQ: f32 = 440.0;
        const DRIVE: f32 = 0.5;
        // Skip FIR warmup + DC-blocker settling (R=0.999 → τ≈1000 samples;
        // 6000 samples ≈ 6τ → <0.25% residual DC).
        const SKIP: usize = 6_000;

        let run = |enabled: bool| -> Vec<f32> {
            let mut sat = Saturation::new(SR, 1);
            sat.set_params(&make_params(enabled, DRIVE, 1.0));
            let mut buf: Vec<f32> = (0..N)
                .map(|i| (2.0 * std::f32::consts::PI * FREQ / SR * i as f32).sin() * 0.5)
                .collect();
            sat.process(&mut buf, 1);
            buf
        };

        let out_on = run(true);
        let out_off = run(false);

        let rms_on = rms(&out_on[SKIP..]);
        let rms_off = rms(&out_off[SKIP..]);

        // Within ±2 dB means ratio within [10^(-2/20), 10^(2/20)] ≈ [0.794, 1.259].
        let ratio = rms_on / rms_off.max(1e-9);
        assert!(
            ratio > 0.794 && ratio < 1.259,
            "makeup should keep loudness within ±2 dB: rms_on={rms_on:.5}, rms_off={rms_off:.5}, ratio={ratio:.4}"
        );
    }

    // ── stays_bounded ────────────────────────────────────────────────────────

    #[test]
    fn stays_bounded() {
        let mut sat = Saturation::new(SR, 2);
        sat.set_params(&make_params(true, 1.0, 1.0));

        // Hostile input: clipped square wave and large impulse
        let mut buf: Vec<f32> = (0..2048)
            .flat_map(|i| {
                let v = if i % 2 == 0 { 10.0 } else { -10.0 };
                [v, -v]
            })
            .collect();
        sat.process(&mut buf, 2);

        assert!(
            buf.iter().all(|&x| x.abs() <= 4.0),
            "output must stay within ±4: max={}",
            buf.iter().map(|x| x.abs()).fold(0.0f32, f32::max)
        );
    }

    // ── antialias_smoke ───────────────────────────────────────────────────────
    // Optional: a 15 kHz tone at high drive stays bounded and fundamental survives.

    #[test]
    fn antialias_smoke() {
        const N: usize = 8192;
        const FREQ: f32 = 15_000.0;

        let mut sat = Saturation::new(SR, 1);
        sat.set_params(&make_params(true, 1.0, 1.0));

        let input: Vec<f32> = (0..N)
            .map(|i| (2.0 * std::f32::consts::PI * FREQ / SR * i as f32).sin() * 0.5)
            .collect();
        let mut buf = input.clone();
        sat.process(&mut buf, 1);

        // Output must be bounded.
        assert!(buf.iter().all(|&x| x.abs() <= 4.0), "output must stay ≤4");

        // The 15 kHz fundamental must survive (FFT magnitude > threshold).
        let lat = Oversampler4x::new(SR).latency_samples();
        let skip = lat * 2 + 64;
        let seg_len = N - skip;
        let mut seg = buf[skip..skip + seg_len].to_vec();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(seg_len);
        let mut spec = fft.make_output_vec();
        fft.process(&mut seg, &mut spec).unwrap();
        let bin_15k = (FREQ / SR * seg_len as f32).round() as usize;
        let mag = spec[bin_15k.min(spec.len() - 1)].norm() / seg_len as f32;
        assert!(mag > 0.01, "15 kHz fundamental must survive saturation: mag={mag:.6}");
    }
}
