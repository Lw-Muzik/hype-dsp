//! Real-time convolution (impulse-response) stage — uniform-partitioned
//! overlap-save FFT convolution. Per-block cost is constant and bounded
//! (one FFT + K complex multiply-accumulates + one IFFT, where K is the
//! capped partition count), so long IRs never stall the audio thread.
//!
//! The IR is prepared off-thread (decode/resample/normalize/partition/FFT) into
//! a [`PreparedIr`] and published to the live stage by a lock-free [`ArcSwap`].

use std::sync::Arc;

use arc_swap::ArcSwap;
use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use crate::{AudioProcessor, ProcessorParams};

/// Partition / hop size in samples. Latency of the stage = this many samples
/// (~5.3 ms @ 48 kHz) — imperceptible for a player.
pub const CONV_BLOCK: usize = 256;
/// FFT length for overlap-save = 2 · CONV_BLOCK.
pub const CONV_FFT: usize = 512;
/// IRs longer than this are truncated, bounding CPU and memory.
pub const MAX_IR_SECONDS: f32 = 4.0;

/// Number of complex bins in a real FFT of length [`CONV_FFT`].
const BINS: usize = CONV_FFT / 2 + 1;

/// One channel of a prepared impulse response: the forward FFT of each
/// zero-padded `CONV_BLOCK` partition.
#[derive(Clone)]
pub struct PreparedIrChannel {
    pub partitions: Vec<Vec<Complex<f32>>>,
}

/// Per-channel real-time convolution state. Owns FFT machinery, the
/// frequency-domain delay line (FDL) of past input spectra, and the streaming
/// FIFOs that decouple the engine's (variable) block size from `CONV_BLOCK`.
struct MonoConvolver {
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    /// Forward-FFT scratch (length CONV_FFT): [prev_block | new_block].
    window: Vec<f32>,
    /// FDL ring of input spectra, length = max_partitions (pre-allocated).
    fdl: Vec<Vec<Complex<f32>>>,
    fdl_pos: usize,
    /// Complex accumulator (length BINS) for the multiply-accumulate.
    acc: Vec<Complex<f32>>,
    /// IFFT output scratch (length CONV_FFT).
    ifft_out: Vec<f32>,
    /// FFT input scratch reused by realfft (length CONV_FFT).
    fft_in: Vec<f32>,
    /// FFT output scratch (length BINS).
    fft_out: Vec<Complex<f32>>,
    /// Fixed input accumulator: fills to CONV_BLOCK then triggers one block.
    /// A fixed array (not a Vec) so no heap allocation ever happens in process().
    accum: [f32; CONV_BLOCK],
    accum_len: usize,
    /// Output FIFO of processed (wet) samples, primed with CONV_BLOCK zeros so
    /// there is always >= input-count available to pop (gives CONV_BLOCK latency).
    /// Reserved capacity is never exceeded at steady state, so no reallocation.
    out_fifo: std::collections::VecDeque<f32>,
}

impl MonoConvolver {
    fn new(max_partitions: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CONV_FFT);
        let ifft = planner.plan_fft_inverse(CONV_FFT);
        let mut out_fifo = std::collections::VecDeque::with_capacity(CONV_BLOCK * 64);
        // Prime with CONV_BLOCK zeros = the convolution's inherent latency.
        for _ in 0..CONV_BLOCK {
            out_fifo.push_back(0.0);
        }
        Self {
            fft,
            ifft,
            window: vec![0.0; CONV_FFT],
            fdl: vec![vec![Complex::new(0.0, 0.0); BINS]; max_partitions.max(1)],
            fdl_pos: 0,
            acc: vec![Complex::new(0.0, 0.0); BINS],
            ifft_out: vec![0.0; CONV_FFT],
            fft_in: vec![0.0; CONV_FFT],
            fft_out: vec![Complex::new(0.0, 0.0); BINS],
            accum: [0.0; CONV_BLOCK],
            accum_len: 0,
            out_fifo,
        }
    }

    /// Process one full CONV_BLOCK of input through the partitioned IR,
    /// appending CONV_BLOCK wet samples to `out_fifo`.
    fn process_block(&mut self, block: &[f32; CONV_BLOCK], ir: &PreparedIrChannel) {
        // Cloning these Arc handles is a refcount bump, NOT a heap allocation —
        // it sidesteps the borrow checker (shared `*self.fft` + mutable scratch
        // fields in one call) while staying real-time safe.
        let fft = self.fft.clone();
        let ifft = self.ifft.clone();

        // window = [previous block | this block]; shift the previous half down.
        self.window.copy_within(CONV_BLOCK..CONV_FFT, 0);
        self.window[CONV_BLOCK..CONV_FFT].copy_from_slice(block);

        // Forward FFT of the 2B window into the FDL slot at fdl_pos.
        self.fft_in.copy_from_slice(&self.window);
        fft.process(&mut self.fft_in, &mut self.fft_out).expect("forward fft");
        self.fdl[self.fdl_pos].copy_from_slice(&self.fft_out);

        // Multiply-accumulate: acc = Σ_k FDL[fdl_pos - k] · IR.partitions[k].
        for a in self.acc.iter_mut() {
            *a = Complex::new(0.0, 0.0);
        }
        let k_max = ir.partitions.len().min(self.fdl.len());
        for k in 0..k_max {
            let idx = (self.fdl_pos + self.fdl.len() - k) % self.fdl.len();
            let x = &self.fdl[idx];
            let h = &ir.partitions[k];
            for b in 0..BINS {
                self.acc[b] += x[b] * h[b];
            }
        }
        self.fdl_pos = (self.fdl_pos + 1) % self.fdl.len();

        // IFFT; overlap-save keeps the SECOND half (valid linear-convolution
        // part). realfft's inverse is unnormalized → divide by CONV_FFT. We pass
        // `&mut self.acc` directly (no clone): acc is recomputed next block, so
        // letting the IFFT consume it as scratch is fine — and allocation-free.
        // realfft's c2r requires the DC and Nyquist bins to be purely real; FP
        // rounding in the MAC can leave a tiny imaginary part, so zero them.
        self.acc[0].im = 0.0;
        self.acc[BINS - 1].im = 0.0;
        ifft.process(&mut self.acc, &mut self.ifft_out).expect("inverse fft");
        let norm = 1.0 / CONV_FFT as f32;
        for &v in &self.ifft_out[CONV_BLOCK..CONV_FFT] {
            self.out_fifo.push_back(v * norm);
        }
    }

    /// Stream arbitrary-length `input` → `out` (same length). `out[i]` is the
    /// wet (convolved) sample, delayed by CONV_BLOCK relative to `input[i]`.
    /// Allocation-free: a fixed stack array carries each full block.
    fn process(&mut self, input: &[f32], out: &mut [f32], ir: &PreparedIrChannel) {
        for (i, &x) in input.iter().enumerate() {
            self.accum[self.accum_len] = x;
            self.accum_len += 1;
            if self.accum_len == CONV_BLOCK {
                // `[f32; CONV_BLOCK]` is Copy → this is a stack copy, not a heap
                // allocation; it frees `self` for the &mut call below.
                let block = self.accum;
                self.accum_len = 0;
                self.process_block(&block, ir);
            }
            out[i] = self.out_fifo.pop_front().unwrap_or(0.0);
        }
    }
}

/// A fully prepared impulse response, ready for real-time convolution.
pub struct PreparedIr {
    pub channels: usize,
    pub l: PreparedIrChannel,
    pub r: Option<PreparedIrChannel>,
    pub num_partitions: usize,
    pub seconds: f32,
    pub truncated: bool,
}

/// Maximum partition count for the engine sample rate — sizes the FDL ring.
pub fn max_partitions(target_sr: f32) -> usize {
    ((MAX_IR_SECONDS * target_sr) as usize)
        .div_ceil(CONV_BLOCK)
        .max(1)
}

/// Linear-interpolating resampler for one mono channel. Adequate for IRs;
/// runs off the audio thread so quality/cost trade-offs are non-critical.
fn resample_linear(input: &[f32], src_sr: f32, dst_sr: f32) -> Vec<f32> {
    if (src_sr - dst_sr).abs() < f32::EPSILON || input.is_empty() {
        return input.to_vec();
    }
    let ratio = dst_sr as f64 / src_sr as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let a = input.get(i0).copied().unwrap_or(0.0);
        let b = input.get(i0 + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Partition a time-domain IR channel into forward-FFT'd `CONV_BLOCK` blocks.
fn partition_channel(ir: &[f32], fft: &Arc<dyn RealToComplex<f32>>) -> PreparedIrChannel {
    let num = ir.len().div_ceil(CONV_BLOCK).max(1);
    let mut partitions = Vec::with_capacity(num);
    for p in 0..num {
        let mut buf = vec![0.0f32; CONV_FFT];
        let start = p * CONV_BLOCK;
        let end = (start + CONV_BLOCK).min(ir.len());
        if start < ir.len() {
            buf[..end - start].copy_from_slice(&ir[start..end]);
        }
        let mut spec = fft.make_output_vec();
        fft.process(&mut buf, &mut spec).expect("ir partition fft");
        partitions.push(spec);
    }
    PreparedIrChannel { partitions }
}

impl PreparedIr {
    /// Build a prepared IR from interleaved `samples`. Heavy — call OFF the
    /// audio thread. Steps: de-interleave → resample to `target_sr` → cap to
    /// MAX_IR_SECONDS → L2-energy-normalize (per the combined IR) → partition+FFT.
    pub fn build(samples: &[f32], src_channels: usize, src_sr: f32, target_sr: f32) -> PreparedIr {
        let src_channels = src_channels.max(1);
        let stereo = src_channels >= 2;

        // De-interleave into one or two mono channels.
        debug_assert_eq!(
            samples.len() % src_channels,
            0,
            "IR sample buffer length must be a multiple of src_channels"
        );
        let frames = samples.len() / src_channels;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(if stereo { frames } else { 0 });
        for f in 0..frames {
            left.push(samples[f * src_channels]);
            if stereo {
                right.push(samples[f * src_channels + 1]);
            }
        }

        // Resample to engine rate.
        let mut left = resample_linear(&left, src_sr, target_sr);
        let mut right = if stereo {
            resample_linear(&right, src_sr, target_sr)
        } else {
            Vec::new()
        };

        // Length cap.
        let cap = (MAX_IR_SECONDS * target_sr) as usize;
        let truncated = left.len() > cap;
        if truncated {
            left.truncate(cap);
            if stereo {
                right.truncate(cap);
            }
        }
        let seconds = left.len() as f32 / target_sr;

        // L2-energy normalization across the whole IR (both channels) → unity
        // energy, so swapping IRs keeps perceived loudness stable.
        let mut energy = 0.0f64;
        for &v in &left {
            energy += (v as f64) * (v as f64);
        }
        for &v in &right {
            energy += (v as f64) * (v as f64);
        }
        let norm = if energy > 1e-20 {
            (1.0 / energy.sqrt()) as f32
        } else {
            1.0
        };
        for v in left.iter_mut() {
            *v *= norm;
        }
        for v in right.iter_mut() {
            *v *= norm;
        }

        // Partition + FFT.
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CONV_FFT);
        let l = partition_channel(&left, &fft);
        let num_partitions = l.partitions.len();
        let r = if stereo {
            Some(partition_channel(&right, &fft))
        } else {
            None
        };

        PreparedIr {
            channels: if stereo { 2 } else { 1 },
            l,
            r,
            num_partitions,
            seconds,
            truncated,
        }
    }
}

/// Lock-free handle to the active prepared IR. The command thread `store`s a new
/// IR; the audio thread `load`s it once per block. `None` = no IR (identity).
pub type IrSlot = Arc<ArcSwap<Option<Arc<PreparedIr>>>>;

/// Create an empty IR slot (no IR loaded).
pub fn empty_ir_slot() -> IrSlot {
    Arc::new(ArcSwap::from_pointee(None))
}

/// Convolution (impulse-response) processing stage.
pub struct Convolver {
    sample_rate: f32,
    enabled: bool,
    wet: f32,
    gain: f32, // linear, from ir_gain_db
    slot: IrSlot,
    left: MonoConvolver,
    right: MonoConvolver,
    /// Scratch for one channel's deinterleaved input/output (sized in prepare).
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
}

impl Convolver {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self::with_slot(sample_rate, channels, empty_ir_slot())
    }

    pub fn with_slot(sample_rate: f32, _channels: usize, slot: IrSlot) -> Self {
        let mp = max_partitions(sample_rate);
        Self {
            sample_rate,
            enabled: false,
            wet: 1.0,
            gain: 1.0,
            slot,
            left: MonoConvolver::new(mp),
            right: MonoConvolver::new(mp),
            in_l: Vec::new(),
            in_r: Vec::new(),
            out_l: Vec::new(),
            out_r: Vec::new(),
        }
    }

    /// A clone of the IR slot, so the engine can publish IRs to this stage.
    pub fn slot(&self) -> IrSlot {
        self.slot.clone()
    }

    fn ensure_scratch(&mut self, frames: usize) {
        if self.in_l.len() < frames {
            self.in_l.resize(frames, 0.0);
            self.in_r.resize(frames, 0.0);
            self.out_l.resize(frames, 0.0);
            self.out_r.resize(frames, 0.0);
        }
    }
}

impl AudioProcessor for Convolver {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        self.sample_rate = sample_rate;
        let mp = max_partitions(sample_rate);
        self.left = MonoConvolver::new(mp);
        self.right = MonoConvolver::new(mp);
        // Pre-size scratch for a generous block; process() grows it off the RT
        // path only if a larger block ever arrives (rare; bounded by device).
        self.in_l.clear();
        self.in_r.clear();
        self.out_l.clear();
        self.out_r.clear();
        self.ensure_scratch(4096);
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || self.wet <= 0.0 || channels == 0 {
            return;
        }
        let ir_guard = self.slot.load();
        let Some(ir) = ir_guard.as_ref() else {
            return; // no IR → identity
        };
        let frames = buffer.len() / channels;
        if frames == 0 {
            return;
        }
        self.ensure_scratch(frames);

        // De-interleave (L and, if present, R).
        let stereo = channels >= 2;
        for f in 0..frames {
            self.in_l[f] = buffer[f * channels];
            self.in_r[f] = if stereo { buffer[f * channels + 1] } else { buffer[f * channels] };
        }

        // Convolve. Mono IR → same partitions for both channels.
        let ir_r = ir.r.as_ref().unwrap_or(&ir.l);
        self.left.process(&self.in_l[..frames], &mut self.out_l[..frames], &ir.l);
        self.right.process(&self.in_r[..frames], &mut self.out_r[..frames], ir_r);

        // Wet/dry mix + gain + bounded clamp. NOTE: dry is mixed with the
        // CONV_BLOCK-delayed wet; the small latency is imperceptible and the
        // wet/dry blend stays phase-stable for correction/reverb IRs.
        let wet = self.wet * self.gain;
        let dry = 1.0 - self.wet;
        for f in 0..frames {
            let dl = self.in_l[f];
            let wl = self.out_l[f];
            let ml = (dl * dry + wl * wet).clamp(-4.0, 4.0);
            buffer[f * channels] = ml;
            if stereo {
                let dr = self.in_r[f];
                let wr = self.out_r[f];
                let mr = (dr * dry + wr * wet).clamp(-4.0, 4.0);
                buffer[f * channels + 1] = mr;
            }
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        let c = &params.convolver;
        self.enabled = c.enabled;
        self.wet = c.wet_dry.clamp(0.0, 1.0);
        self.gain = 10f32.powf(c.ir_gain_db / 20.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{ConvolverState, EngineState};

    fn conv_state(enabled: bool, wet: f32) -> EngineState {
        EngineState {
            convolver: ConvolverState { enabled, wet_dry: wet, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn disabled_is_identity() {
        let mut c = Convolver::new(48_000.0, 2);
        c.set_params(&EngineState::default()); // disabled
        let input = vec![0.5, -0.3, 0.2, 0.4, -0.1, 0.9];
        let mut buf = input.clone();
        c.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn enabled_without_ir_is_identity() {
        let mut c = Convolver::new(48_000.0, 2);
        c.set_params(&conv_state(true, 1.0)); // enabled but no IR published
        let input = vec![0.5, -0.3, 0.2, 0.4];
        let mut buf = input.clone();
        c.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn stays_bounded_with_loud_ir() {
        let slot = empty_ir_slot();
        let mut c = Convolver::with_slot(48_000.0, 2, slot.clone());
        // A long, hot IR — energy normalization + clamp must keep it bounded.
        let h = vec![0.9f32; 4000];
        slot.store(Arc::new(Some(Arc::new(PreparedIr::build(&h, 1, 48_000.0, 48_000.0)))));
        c.set_params(&conv_state(true, 1.0));
        let mut buf: Vec<f32> = (0..48_000 * 2).map(|i| if i % 2 == 0 { 0.9 } else { -0.9 }).collect();
        c.process(&mut buf, 2);
        assert!(buf.iter().all(|&x| x.abs() <= 4.0), "convolver must stay bounded");
    }

    /// Build a single-channel prepared IR from a raw time-domain IR by
    /// partitioning + forward-FFT (mirrors PreparedIr::build, Task 3).
    fn prepare_channel(ir: &[f32]) -> PreparedIrChannel {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CONV_FFT);
        let num = ir.len().div_ceil(CONV_BLOCK).max(1);
        let mut partitions = Vec::with_capacity(num);
        for p in 0..num {
            let mut buf = vec![0.0f32; CONV_FFT];
            let start = p * CONV_BLOCK;
            let end = (start + CONV_BLOCK).min(ir.len());
            // Partition occupies the FIRST half; second half stays zero.
            buf[..end - start].copy_from_slice(&ir[start..end]);
            let mut spec = fft.make_output_vec();
            fft.process(&mut buf, &mut spec).unwrap();
            partitions.push(spec);
        }
        PreparedIrChannel { partitions }
    }

    /// Direct time-domain convolution reference.
    fn direct_conv(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0f32; x.len()];
        for n in 0..x.len() {
            let mut acc = 0.0;
            for (k, &hk) in h.iter().enumerate() {
                if n >= k {
                    acc += x[n - k] * hk;
                }
            }
            y[n] = acc;
        }
        y
    }

    #[test]
    fn unit_impulse_ir_is_delayed_passthrough() {
        // IR = [1.0] → output equals input, delayed by CONV_BLOCK.
        let ir = prepare_channel(&[1.0]);
        let mut mc = MonoConvolver::new(1);
        let x: Vec<f32> = (0..CONV_BLOCK * 4).map(|i| (i as f32 * 0.05).sin()).collect();
        let mut y = vec![0.0; x.len()];
        mc.process(&x, &mut y, &ir);
        for i in 0..(x.len() - CONV_BLOCK) {
            assert!(
                (y[i + CONV_BLOCK] - x[i]).abs() < 1e-4,
                "delayed passthrough mismatch at {i}: {} vs {}",
                y[i + CONV_BLOCK],
                x[i]
            );
        }
    }

    #[test]
    fn matches_direct_convolution() {
        // Short IR; compare against a direct time-domain convolution (shifted by latency).
        let h: Vec<f32> = (0i32..600).map(|i| 0.9f32.powi(i) * if i % 2 == 0 { 1.0 } else { -0.5 }).collect();
        let ir = prepare_channel(&h);
        let max_parts = h.len().div_ceil(CONV_BLOCK);
        let mut mc = MonoConvolver::new(max_parts);
        let x: Vec<f32> = (0..CONV_BLOCK * 10).map(|i| (i as f32 * 0.03).sin()).collect();
        let mut y = vec![0.0; x.len()];
        mc.process(&x, &mut y, &ir);
        let reference = direct_conv(&x, &h);
        for i in 0..(x.len() - CONV_BLOCK) {
            assert!(
                (y[i + CONV_BLOCK] - reference[i]).abs() < 1e-2,
                "conv mismatch at {i}: {} vs {}",
                y[i + CONV_BLOCK],
                reference[i]
            );
        }
    }

    #[test]
    fn build_mono_unit_impulse_normalized_passthrough() {
        // A single-sample IR, energy-normalized to L2=1, is still 1.0 → passthrough.
        let ir = PreparedIr::build(&[0.5], 1, 48_000.0, 48_000.0);
        assert_eq!(ir.channels, 1);
        assert!(ir.r.is_none());
        assert_eq!(ir.num_partitions, 1);
        let mut mc = MonoConvolver::new(ir.num_partitions);
        let x: Vec<f32> = (0..CONV_BLOCK * 3).map(|i| (i as f32 * 0.07).sin()).collect();
        let mut y = vec![0.0; x.len()];
        mc.process(&x, &mut y, &ir.l);
        for i in 0..(x.len() - CONV_BLOCK) {
            assert!((y[i + CONV_BLOCK] - x[i]).abs() < 1e-4);
        }
    }

    #[test]
    fn build_caps_length() {
        // 10 s @ 48k truncates to MAX_IR_SECONDS.
        let n = 48_000 * 10;
        let samples = vec![0.01f32; n];
        let ir = PreparedIr::build(&samples, 1, 48_000.0, 48_000.0);
        assert!(ir.truncated);
        assert!(ir.seconds <= MAX_IR_SECONDS + 0.001);
        let max_parts = ((MAX_IR_SECONDS * 48_000.0) as usize).div_ceil(CONV_BLOCK);
        assert!(ir.num_partitions <= max_parts);
    }

    #[test]
    fn build_resamples_to_target() {
        // 44.1k IR built for a 48k engine → seconds preserved (within a frame).
        let secs = 0.5;
        let n = (44_100.0 * secs) as usize;
        let samples = vec![0.01f32; n];
        let ir = PreparedIr::build(&samples, 1, 44_100.0, 48_000.0);
        assert!((ir.seconds - secs).abs() < 0.01, "seconds={}", ir.seconds);
    }

    #[test]
    fn build_stereo_has_two_channels() {
        let samples: Vec<f32> = (0..1000).flat_map(|i| [i as f32 * 0.001, -(i as f32) * 0.001]).collect();
        let ir = PreparedIr::build(&samples, 2, 48_000.0, 48_000.0);
        assert_eq!(ir.channels, 2);
        assert!(ir.r.is_some());
        let r = ir.r.as_ref().unwrap();
        assert_ne!(
            ir.l.partitions[0][1].re, r.partitions[0][1].re,
            "stereo channels must carry independent content"
        );
    }

    #[test]
    fn chunking_invariance() {
        // Processing in odd-sized chunks yields the same result as one call.
        let h: Vec<f32> = (0..300).map(|i| (i as f32 * 0.1).cos()).collect();
        let ir = prepare_channel(&h);
        let parts = h.len().div_ceil(CONV_BLOCK);
        let x: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.02).sin()).collect();

        let mut a = MonoConvolver::new(parts);
        let mut ya = vec![0.0; x.len()];
        a.process(&x, &mut ya, &ir);

        let mut b = MonoConvolver::new(parts);
        let mut yb = vec![0.0; x.len()];
        let mut off = 0;
        for chunk in [37usize, 100, 1, 256, 511].iter().cycle() {
            if off >= x.len() { break; }
            let end = (off + chunk).min(x.len());
            b.process(&x[off..end], &mut yb[off..end], &ir);
            off = end;
        }
        for i in 0..x.len() {
            assert!((ya[i] - yb[i]).abs() < 1e-5, "chunking differs at {i}");
        }
    }
}
