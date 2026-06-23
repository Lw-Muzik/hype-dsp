//! Real-time convolution (impulse-response) stage — uniform-partitioned
//! overlap-save FFT convolution. Per-block cost is constant and bounded
//! (one FFT + K complex multiply-accumulates + one IFFT, where K is the
//! capped partition count), so long IRs never stall the audio thread.
//!
//! The IR is prepared off-thread (decode/resample/normalize/partition/FFT) into
//! a [`PreparedIr`] and published to the live stage by a lock-free [`ArcSwap`].

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

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
// TODO: remove this allow once Task 4 wires MonoConvolver into the Convolver stage.
#[allow(dead_code)]
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

// TODO: remove this allow once Task 4 wires MonoConvolver into the Convolver stage.
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
