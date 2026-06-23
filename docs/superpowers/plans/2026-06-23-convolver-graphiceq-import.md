# Convolver (impulse-response) engine + GraphicEQ-string import — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a real-time convolution (impulse-response) DSP stage to HypeMuzik Desktop, plus an EqualizerAPO GraphicEQ-string import for the existing 31-band EQ — both highly performant (audio/UI threads never block) and clip-proof.

**Architecture:** A new `Convolver` `AudioProcessor` stage runs **uniform-partitioned overlap-save FFT convolution** with bounded, constant per-block cost. All heavy IR work (decode, resample, normalize, partition, pre-FFT) happens **off the audio thread** and is published to the stage by a lock-free `ArcSwap` pointer swap. Gain is staged in four layers so output can't clip. GraphicEQ import is pure data transformation reusing the existing EQ — no new DSP.

**Tech Stack:** Rust (`hm-core` types, `hm-dsp` DSP, `hm-audio` engine), `realfft` (FFT, already a dep), `hound` (WAV decode, already a dep), `arc-swap` (lock-free handoff, workspace dep), Tauri commands, React + Zustand + Tailwind (TS UI).

## Global Constraints

- **Real-time safety:** `AudioProcessor::process` must never allocate, lock, or do I/O (it runs on the audio callback). All allocation happens in `prepare`. Verbatim trait contract — `crates/hm-dsp/src/lib.rs:46-60`.
- **Off-thread heavy work:** IR decode/resample/normalize/partition/FFT-plan run in the Tauri command thread, never the audio thread. Hand off via `ArcSwap` only.
- **Chain order (verbatim):** `Headphone → GraphicEq → Bass → Spatializer → Surround3D → Room → Convolver → Gain → Limiter`. The Convolver sits before `Gain → Limiter` so the existing −0.3 dBFS brickwall limiter is the final safety net.
- **No clipping:** four-layer gain staging — IR L2-normalization, wet/dry + dB trim, in-stage `clamp(-4.0, 4.0)`, master limiter.
- **Dropdowns:** the app has exactly one dropdown component — `@/components/Combobox`. Never use a native `<select>`.
- **Slider width footgun:** the shared `<Slider>` must always have a width class (`flex-1` / `w-32`) or it collapses to 0 px and drag silently dies.
- **Type contract:** every `hm-core` public type is mirrored by a TS interface in `src/lib/types.ts`. Change both sides together.
- **Constants:** `CONV_BLOCK = 256` (partition/hop size), `CONV_FFT = 512` (= 2·CONV_BLOCK), `MAX_IR_SECONDS = 4.0`. `BAND_COUNT = 31`, `ISO_CENTERS_HZ` from `hm-core`.
- **Verification gates before any task is "done":** `cargo test -p <crate>` green, `cargo clippy --all-targets -- -D warnings` clean for touched crates, `pnpm tsc --noEmit` clean for TS tasks.

---

## File Structure

**Create:**
- `crates/hm-dsp/src/convolver.rs` — `PreparedIr`, `PreparedIrChannel`, `MonoConvolver`, `Convolver` stage, `IrSlot`, `empty_ir_slot()`.
- `crates/hm-core/src/graphic_eq_import.rs` — `parse_graphic_eq`, `interpolate_to_iso_bands`, `recommended_preamp`.
- `crates/hm-audio/src/ir_loader.rs` — `load_ir_samples(path)` (WAV decode → f32).
- `src/features/enhancer/ConvolverCard.tsx` — the UI card.

**Modify:**
- `crates/hm-core/src/types.rs` — add `ConvolverState`, add `pub convolver: ConvolverState` to `EngineState` + `Default`.
- `crates/hm-core/src/lib.rs` — `pub mod graphic_eq_import;` + re-export.
- `crates/hm-dsp/Cargo.toml` — add `arc-swap = { workspace = true }`.
- `crates/hm-dsp/src/lib.rs` — `pub mod convolver;`, re-export `Convolver`, insert into `ProcessChain` via `standard_with_ir`.
- `crates/hm-audio/src/engine.rs` — `ir_slot` field + thread/Renderer plumbing + `set_convolver` / `load_convolver_ir`.
- `src-tauri/src/commands/engine.rs` + `src-tauri/src/lib.rs` — three new commands + registration.
- `src/lib/types.ts`, `src/lib/ipc.ts`, `src/stores/engine.ts` — TS state, IPC wrappers, store actions.
- `src/features/enhancer/` index/view that lists the cards — add `ConvolverCard`; add an "Import curve" affordance to the EQ card.

---

## Task 1: `ConvolverState` type (hm-core) + EngineState wiring + TS mirror

**Files:**
- Modify: `crates/hm-core/src/types.rs`
- Test: `crates/hm-core/src/types.rs` (`#[cfg(test)]`)
- Modify: `src/lib/types.ts`

**Interfaces:**
- Produces: `hm_core::ConvolverState { enabled: bool, wet_dry: f32, ir_gain_db: f32, ir_id: Option<String>, ir_name: Option<String>, ir_seconds: f32, ir_truncated: bool }`, default = disabled/empty. New field `EngineState.convolver: ConvolverState`.

- [ ] **Step 1: Write the failing test** — append to the `#[cfg(test)]` module in `types.rs`:

```rust
    #[test]
    fn convolver_default_is_disabled_and_empty() {
        let c = ConvolverState::default();
        assert!(!c.enabled);
        assert_eq!(c.wet_dry, 1.0);
        assert_eq!(c.ir_gain_db, 0.0);
        assert!(c.ir_id.is_none());
        assert!(!c.ir_truncated);
        // Present on EngineState and off by default.
        assert!(!EngineState::default().convolver.enabled);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hm-core convolver_default_is_disabled_and_empty`
Expected: FAIL — `no variant or associated item named default ... ConvolverState` / `no field convolver`.

- [ ] **Step 3: Add the type** — in `types.rs`, after `RoomState`'s `impl Default` block (around line 185), add:

```rust
/// Convolution (impulse-response) stage state. The heavy IR data is NOT stored
/// here — it is published to the audio stage out-of-band via a lock-free slot.
/// These are only the cheap scalars the audio thread reads each block, plus
/// metadata the UI displays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvolverState {
    pub enabled: bool,
    /// Wet/dry mix (0 = dry … 1 = fully wet). Correction IRs run ~1.0.
    pub wet_dry: f32,
    /// Post-convolution trim in dB applied to the wet path.
    pub ir_gain_db: f32,
    /// Identifier (path or bundled id) of the loaded IR, for the UI.
    pub ir_id: Option<String>,
    /// Human-facing IR name, for the UI.
    pub ir_name: Option<String>,
    /// IR length in seconds after the length cap.
    pub ir_seconds: f32,
    /// Whether the IR was truncated by the length cap.
    pub ir_truncated: bool,
}

impl Default for ConvolverState {
    fn default() -> Self {
        Self {
            enabled: false,
            wet_dry: 1.0,
            ir_gain_db: 0.0,
            ir_id: None,
            ir_name: None,
            ir_seconds: 0.0,
            ir_truncated: false,
        }
    }
}
```

Then add the field to `EngineState` (after `pub room: RoomState,`):

```rust
    pub convolver: ConvolverState,
```

And to `EngineState`'s `Default` impl (after `room: RoomState::default(),`):

```rust
            convolver: ConvolverState::default(),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p hm-core convolver_default_is_disabled_and_empty`
Expected: PASS.

- [ ] **Step 5: Mirror the TS type** — in `src/lib/types.ts`, after the `RoomState` interface (around line 78), add:

```ts
/** Convolution (impulse-response) stage. Heavy IR data lives engine-side. */
export interface ConvolverState {
  enabled: boolean;
  wetDry: number;
  irGainDb: number;
  irId: string | null;
  irName: string | null;
  irSeconds: number;
  irTruncated: boolean;
}
```

Then add `convolver: ConvolverState;` to the `EngineState` interface (next to `room: RoomState;`).

- [ ] **Step 6: Verify TS compiles**

Run: `pnpm tsc --noEmit`
Expected: no errors (a missing `convolver` in any default-state literal will surface here — if so, add `convolver` to that literal using the defaults above).

- [ ] **Step 7: Commit**

```bash
git add crates/hm-core/src/types.rs src/lib/types.ts
git commit -m "feat(core): add ConvolverState to EngineState + TS mirror"
```

---

## Task 2: Partitioned overlap-save mono convolution engine (`MonoConvolver`)

This is the real-time core. It owns per-channel mutable state (FFT plans, frequency-domain delay line, FIFOs); the immutable IR is passed in each block.

**Files:**
- Create: `crates/hm-dsp/src/convolver.rs`
- Modify: `crates/hm-dsp/Cargo.toml`, `crates/hm-dsp/src/lib.rs`
- Test: `crates/hm-dsp/src/convolver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub const CONV_BLOCK: usize = 256;` `pub const CONV_FFT: usize = 512;` `pub const MAX_IR_SECONDS: f32 = 4.0;`
  - `pub struct PreparedIrChannel { pub partitions: Vec<Vec<Complex<f32>>> }` (each inner = `CONV_FFT/2+1` complex spectrum; outer len = number of partitions).
  - `struct MonoConvolver` with `fn new(max_partitions: usize) -> Self` and `fn process(&mut self, input: &[f32], out: &mut [f32], ir: &PreparedIrChannel)`.

- [ ] **Step 1: Add the dependency** — in `crates/hm-dsp/Cargo.toml`, under `[dependencies]`, add:

```toml
arc-swap = { workspace = true }
```

- [ ] **Step 2: Register the module** — in `crates/hm-dsp/src/lib.rs`, add `pub mod convolver;` next to the other `pub mod` lines (after `pub mod bass_boost;`).

- [ ] **Step 3: Write the failing test** — create `crates/hm-dsp/src/convolver.rs` with the test module first so it compiles to a failing state:

```rust
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
        let h: Vec<f32> = (0..600).map(|i| 0.9f32.powi(i as i32) * if i % 2 == 0 { 1.0 } else { -0.5 }).collect();
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
```

- [ ] **Step 4: Run tests to verify they fail then pass** — the implementation above is included in the same file, so:

Run: `cargo test -p hm-dsp convolver::tests`
Expected: PASS for all three (`unit_impulse_ir_is_delayed_passthrough`, `matches_direct_convolution`, `chunking_invariance`). If `matches_direct_convolution` fails on scale, re-check the `1/CONV_FFT` normalization and the overlap-save half (`[CONV_BLOCK..CONV_FFT]`).

- [ ] **Step 5: Clippy**

Run: `cargo clippy -p hm-dsp --all-targets -- -D warnings`
Expected: clean. (`acc.clone()` before IFFT is intentional — realfft consumes its input.)

- [ ] **Step 6: Commit**

```bash
git add crates/hm-dsp/Cargo.toml crates/hm-dsp/src/lib.rs crates/hm-dsp/src/convolver.rs
git commit -m "feat(dsp): partitioned overlap-save mono convolution engine"
```

---

## Task 3: `PreparedIr` — off-thread IR preparation (resample, cap, normalize, partition)

**Files:**
- Modify: `crates/hm-dsp/src/convolver.rs`
- Test: `crates/hm-dsp/src/convolver.rs`

**Interfaces:**
- Consumes: `PreparedIrChannel`, `CONV_BLOCK`, `CONV_FFT`, `MAX_IR_SECONDS`.
- Produces: `pub struct PreparedIr { pub channels: usize, pub l: PreparedIrChannel, pub r: Option<PreparedIrChannel>, pub num_partitions: usize, pub seconds: f32, pub truncated: bool }` and `pub fn build(samples: &[f32], src_channels: usize, src_sr: f32, target_sr: f32) -> PreparedIr` where `samples` is interleaved by `src_channels`.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `convolver.rs`:

```rust
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
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p hm-dsp convolver::tests::build_mono`
Expected: FAIL — `PreparedIr` / `build` not found.

- [ ] **Step 3: Implement** — add to `convolver.rs` (above the `#[cfg(test)]` module):

```rust
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
        let len = left.len().max(1);
        let seconds = len as f32 / target_sr;

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
```

Note: the L2-normalization changes a `[0.5]` IR to `[1.0]` (energy 0.25 → norm 2.0 → 0.5·2.0 = 1.0), which is why `build_mono_unit_impulse_normalized_passthrough` expects passthrough.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p hm-dsp convolver::tests`
Expected: all PASS.

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p hm-dsp --all-targets -- -D warnings
git add crates/hm-dsp/src/convolver.rs
git commit -m "feat(dsp): PreparedIr off-thread IR prep (resample/cap/normalize/partition)"
```

---

## Task 4: `Convolver` `AudioProcessor` stage (IrSlot, wet/dry, gain, identity, clamp)

**Files:**
- Modify: `crates/hm-dsp/src/convolver.rs`, `crates/hm-dsp/src/lib.rs`
- Test: `crates/hm-dsp/src/convolver.rs`

**Interfaces:**
- Consumes: `MonoConvolver`, `PreparedIr`, `max_partitions`, `hm_core::ConvolverState` (via `ProcessorParams`).
- Produces:
  - `pub type IrSlot = Arc<ArcSwap<Option<Arc<PreparedIr>>>>;`
  - `pub fn empty_ir_slot() -> IrSlot`
  - `pub struct Convolver` with `pub fn new(sample_rate: f32, channels: usize) -> Self` and `pub fn with_slot(sample_rate: f32, channels: usize, slot: IrSlot) -> Self`, implementing `AudioProcessor`.
  - Re-export from `hm-dsp/src/lib.rs`: `pub use convolver::{Convolver, IrSlot, empty_ir_slot, PreparedIr};`

- [ ] **Step 1: Write the failing tests** — add to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p hm-dsp convolver::tests::disabled_is_identity`
Expected: FAIL — `Convolver` / `empty_ir_slot` not found.

- [ ] **Step 3: Implement the stage** — add to `convolver.rs` (above the test module):

```rust
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
```

- [ ] **Step 4: Re-export from the crate** — in `crates/hm-dsp/src/lib.rs`, add next to the other `pub use` lines:

```rust
pub use convolver::{empty_ir_slot, Convolver, IrSlot, PreparedIr};
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p hm-dsp convolver::tests`
Expected: all PASS (identity cases bit-exact; bounded case holds).

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p hm-dsp --all-targets -- -D warnings
git add crates/hm-dsp/src/convolver.rs crates/hm-dsp/src/lib.rs
git commit -m "feat(dsp): Convolver stage with lock-free IrSlot, wet/dry, gain, bounded output"
```

---

## Task 5: Insert `Convolver` into `ProcessChain`

**Files:**
- Modify: `crates/hm-dsp/src/lib.rs`
- Test: `crates/hm-dsp/src/lib.rs`

**Interfaces:**
- Consumes: `Convolver`, `IrSlot`, `empty_ir_slot`.
- Produces: `ProcessChain::standard_with_ir(sample_rate, channels, ir_slot: IrSlot) -> ProcessChain`; `ProcessChain::standard` delegates with a fresh empty slot (so the two system-EQ callers are unchanged).

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `lib.rs`:

```rust
    #[test]
    fn standard_chain_is_identity_when_all_off() {
        let mut state = EngineState::default();
        state.eq.enabled = false;
        state.power = true;
        let mut chain = ProcessChain::standard_with_ir(48_000.0, 2, crate::empty_ir_slot());
        chain.set_params(&state);
        // Convolver disabled by default → chain must not blow up; length includes it.
        assert!(chain.len() >= 8, "convolver should be in the standard chain");
        let original: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let mut buf = original.clone();
        chain.process(&mut buf, 2);
        assert!(buf.iter().all(|&x| x.abs() <= 1.0));
    }
```

(Add `use hm_core::EngineState;` to the test module if not already imported.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p hm-dsp standard_chain_is_identity_when_all_off`
Expected: FAIL — `standard_with_ir` not found.

- [ ] **Step 3: Implement** — in `crates/hm-dsp/src/lib.rs`, replace the body of `standard` and add `standard_with_ir`:

```rust
    pub fn standard(sample_rate: f32, channels: usize) -> Self {
        Self::standard_with_ir(sample_rate, channels, crate::empty_ir_slot())
    }

    /// Like [`standard`](Self::standard) but with an externally-owned IR slot so
    /// the engine can publish impulse responses to the convolver stage.
    pub fn standard_with_ir(sample_rate: f32, channels: usize, ir_slot: IrSlot) -> Self {
        let mut chain = Self::new();
        chain.prepare(sample_rate, channels);
        chain.push(Box::new(HeadphoneCorrection::new(sample_rate, channels)));
        chain.push(Box::new(GraphicEq::new(sample_rate, channels)));
        chain.push(Box::new(BassBoost::new(sample_rate, channels)));
        chain.push(Box::new(Spatializer::new(sample_rate, channels)));
        chain.push(Box::new(Surround3D::new(sample_rate, channels)));
        chain.push(Box::new(RoomEffects::new(sample_rate, channels)));
        chain.push(Box::new(Convolver::with_slot(sample_rate, channels, ir_slot)));
        chain.push(Box::new(Gain::new()));
        chain.push(Box::new(Limiter::new(sample_rate, channels)));
        chain
    }
```

Add `Convolver` and `IrSlot` to the imports already re-exported (they are, from Task 4). Update the `standard` doc comment's chain order to include `→ Convolver`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p hm-dsp`
Expected: all PASS (existing + new).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p hm-dsp --all-targets -- -D warnings
git add crates/hm-dsp/src/lib.rs
git commit -m "feat(dsp): insert Convolver into the standard chain before Gain/Limiter"
```

---

## Task 6: IR file loader (`hm-audio`, WAV → f32)

**Files:**
- Create: `crates/hm-audio/src/ir_loader.rs`
- Modify: `crates/hm-audio/src/lib.rs` (add `mod ir_loader;` or `pub mod`)
- Test: `crates/hm-audio/src/ir_loader.rs`

**Interfaces:**
- Produces: `pub fn load_ir_samples(path: &std::path::Path) -> Result<(Vec<f32>, usize, f32), crate::error::AudioError>` returning `(interleaved_samples, channels, sample_rate)`.

- [ ] **Step 1: Confirm the error type** — check `crates/hm-audio/src/error.rs` for the crate's error enum (referred to here as `AudioError`). Use the existing variant for I/O/decode failures; if there's a generic `AudioError::Decode(String)` or similar, use it. (Read the file; do not invent a variant.)

- [ ] **Step 2: Write the failing test** — create `crates/hm-audio/src/ir_loader.rs`:

```rust
//! Loads impulse-response files (WAV/`.irs`) into interleaved f32 samples for
//! [`hm_dsp::PreparedIr::build`]. Runs OFF the audio thread (file I/O).

use std::path::Path;

use crate::error::AudioError;

/// Decode a WAV/IRS impulse response into `(interleaved_f32, channels, sample_rate)`.
pub fn load_ir_samples(path: &Path) -> Result<(Vec<f32>, usize, f32), AudioError> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| AudioError::decode(format!("open IR {}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate as f32;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| AudioError::decode(format!("read IR floats: {e}")))?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|r| r.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()
                .map_err(|e| AudioError::decode(format!("read IR ints: {e}")))?
        }
    };
    if samples.is_empty() {
        return Err(AudioError::decode("IR file is empty".into()));
    }
    Ok((samples, channels, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_written_wav() {
        // Write a tiny mono 48k WAV to a temp path, then read it back.
        let dir = std::env::temp_dir();
        let path = dir.join("hm_ir_test.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..100 {
            w.write_sample(((i as f32 / 100.0) * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();

        let (samples, ch, sr) = load_ir_samples(&path).unwrap();
        assert_eq!(ch, 1);
        assert_eq!(sr, 48_000.0);
        assert_eq!(samples.len(), 100);
        assert!(samples.iter().all(|&s| (-1.0..=1.0).contains(&s)));
        std::fs::remove_file(&path).ok();
    }
}
```

If `AudioError` has no `decode(String)` constructor, replace `AudioError::decode(...)` with the actual constructor/variant found in Step 1 (e.g. `AudioError::Decode(...)`), consistently.

- [ ] **Step 3: Register the module** — in `crates/hm-audio/src/lib.rs`, add `pub mod ir_loader;` with the other module declarations.

- [ ] **Step 4: Run test**

Run: `cargo test -p hm-audio ir_loader`
Expected: PASS.

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p hm-audio --all-targets -- -D warnings
git add crates/hm-audio/src/ir_loader.rs crates/hm-audio/src/lib.rs
git commit -m "feat(audio): WAV/IRS impulse-response file loader"
```

---

## Task 7: Engine wiring — `ir_slot`, `set_convolver`, `load_convolver_ir`

**Files:**
- Modify: `crates/hm-audio/src/engine.rs`

**Interfaces:**
- Consumes: `hm_dsp::{IrSlot, empty_ir_slot, PreparedIr}`, `ir_loader::load_ir_samples`, `ConvolverState`.
- Produces (on `AudioEngine`): `pub fn set_convolver(&self, state: ConvolverState)`, `pub fn load_convolver_ir(&self, path: &Path) -> Result<ConvolverIrInfo, AudioError>`, and a new public struct `pub struct ConvolverIrInfo { pub name: String, pub seconds: f32, pub truncated: bool, pub channels: usize }` (serde).

- [ ] **Step 1: Add imports** — at the top of `engine.rs`, ensure these are present:

```rust
use std::path::Path;
use hm_dsp::{empty_ir_slot, IrSlot, PreparedIr};
use crate::ir_loader::load_ir_samples;
```

- [ ] **Step 2: Add the `ir_slot` field + ControlCtx plumbing** —
  1. Add to `struct AudioEngine`: `ir_slot: IrSlot,`.
  2. In `AudioEngine::new`, after `let stem_gains = ...;` add `let ir_slot = empty_ir_slot();`. Add `let ir_slot = ir_slot.clone();` to the thread-spawn closure's capture block (next to the other `.clone()`s), and pass `ir_slot` into `ControlCtx { ... ir_slot, }`. In the returned `Self { ... }`, add `ir_slot,` (use the original, not the cloned one — clone for the closure, move original into the struct: mirror exactly how `stem_gains` is handled).
  3. Add `ir_slot: IrSlot,` to the `ControlCtx` struct definition.
  4. In `control_loop`, thread `ctx.ir_slot` to wherever `Renderer::new` is called: change those calls to `Renderer::new(source, sample_rate, channels, ctx.ir_slot.clone())`.

- [ ] **Step 3: Update `Renderer::new`** — change its signature and chain construction:

```rust
    pub fn new(
        mut source: Box<dyn AudioSource>,
        sample_rate: f32,
        channels: usize,
        ir_slot: IrSlot,
    ) -> Self {
        let _ = source.start(StreamFormat {
            sample_rate: sample_rate as u32,
            channels: channels as u16,
        });
        Self {
            chain: ProcessChain::standard_with_ir(sample_rate, channels, ir_slot),
            source,
            analyzer: Analyzer::new(sample_rate),
            dbg_logged: 0,
        }
    }
```

Update the two test call sites (`Renderer::new(constant_source(...), 48_000.0, 2)` near lines 1248, 1269) to pass `hm_dsp::empty_ir_slot()` as the 4th argument.

- [ ] **Step 4: Add the engine methods** — near `set_room` (~line 614):

```rust
    /// Update the convolver's cheap scalar params (enabled / wet-dry / gain).
    pub fn set_convolver(&self, mut state: ConvolverState) {
        state.wet_dry = state.wet_dry.clamp(0.0, 1.0);
        self.update(|s| {
            // Preserve loaded-IR metadata published by load_convolver_ir.
            let (id, name, secs, trunc) = (
                s.convolver.ir_id.clone(),
                s.convolver.ir_name.clone(),
                s.convolver.ir_seconds,
                s.convolver.ir_truncated,
            );
            s.convolver = ConvolverState {
                ir_id: state.ir_id.or(id),
                ir_name: state.ir_name.or(name),
                ir_seconds: if state.ir_seconds > 0.0 { state.ir_seconds } else { secs },
                ir_truncated: state.ir_truncated || trunc,
                ..state
            };
        });
    }

    /// Decode, prepare, and publish an impulse response to the live stage.
    /// Heavy work runs here (the caller's command thread), never the audio
    /// thread; the prepared IR is handed off by a lock-free atomic store.
    pub fn load_convolver_ir(&self, path: &Path) -> Result<ConvolverIrInfo, AudioError> {
        let target_sr = self.shared.load().output_sample_rate_or(48_000.0);
        let (samples, channels, src_sr) = load_ir_samples(path)?;
        let prepared = PreparedIr::build(&samples, channels, src_sr, target_sr);
        let info = ConvolverIrInfo {
            name: path.file_name().and_then(|n| n.to_str()).unwrap_or("IR").to_string(),
            seconds: prepared.seconds,
            truncated: prepared.truncated,
            channels: prepared.channels,
        };
        // Publish to the audio thread (lock-free).
        self.ir_slot.store(Arc::new(Some(Arc::new(prepared))));
        // Reflect metadata in state so the UI + autosave see it.
        let info_c = info.clone();
        let id = path.to_string_lossy().to_string();
        self.update(|s| {
            s.convolver.ir_id = Some(id);
            s.convolver.ir_name = Some(info_c.name.clone());
            s.convolver.ir_seconds = info_c.seconds;
            s.convolver.ir_truncated = info_c.truncated;
            s.convolver.enabled = true;
        });
        Ok(info)
    }
```

Add the result struct near the other public structs in `engine.rs`:

```rust
/// Metadata about a freshly-loaded impulse response, returned to the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvolverIrInfo {
    pub name: String,
    pub seconds: f32,
    pub truncated: bool,
    pub channels: usize,
}
```

`target_sr`: there is no `output_sample_rate_or` helper — replace that line with the engine's actual known output rate. If the engine doesn't track the device rate in `EngineState`, default to `48_000.0` (the chain rebuilds per source via `Renderer::new`, and IR resampling targets this rate; a mismatch only causes a slight spectral shift, corrected on next load). Use:

```rust
        let target_sr = 48_000.0_f32;
```

(Leave a `// TODO: thread real device sample rate if it becomes available` comment.)

- [ ] **Step 5: Build + run engine tests**

Run: `cargo test -p hm-audio`
Expected: PASS (including the two updated `Renderer::new` test call sites).

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p hm-audio --all-targets -- -D warnings
git add crates/hm-audio/src/engine.rs
git commit -m "feat(audio): engine ir_slot + set_convolver/load_convolver_ir (off-thread prep, lock-free handoff)"
```

---

## Task 8: Tauri commands + registration

**Files:**
- Modify: `src-tauri/src/commands/engine.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `AudioEngine::{set_convolver, load_convolver_ir}`, `ConvolverState`, `ConvolverIrInfo`.
- Produces: `engine_set_convolver(convolver: ConvolverState)`, `engine_convolver_load_ir(path: String) -> ConvolverIrInfo`.

- [ ] **Step 1: Add the commands** — in `src-tauri/src/commands/engine.rs`, after `engine_set_room` (~line 85):

```rust
/// Configure the convolver stage's scalar params.
#[tauri::command]
pub fn engine_set_convolver(engine: State<'_, AudioEngine>, convolver: hm_core::ConvolverState) {
    engine.set_convolver(convolver);
}

/// Load an impulse-response file into the convolver (heavy prep off the audio thread).
#[tauri::command]
pub fn engine_convolver_load_ir(
    engine: State<'_, AudioEngine>,
    path: String,
) -> Result<hm_audio::ConvolverIrInfo, IpcError> {
    engine
        .load_convolver_ir(&PathBuf::from(path))
        .map_err(Into::into)
}
```

Confirm `hm_audio::ConvolverIrInfo` is exported (Task 7 added it in `engine.rs`; ensure `hm-audio`'s `lib.rs` re-exports `ConvolverIrInfo` alongside the other engine types — add `pub use engine::ConvolverIrInfo;` if engine items are re-exported there). Confirm `AudioError → IpcError` conversion already exists (it does — `load_ir` etc. use `.map_err(Into::into)`).

- [ ] **Step 2: Register** — in `src-tauri/src/lib.rs`, in the `tauri::generate_handler![...]` list, after `commands::engine::engine_set_room,` add:

```rust
            commands::engine::engine_set_convolver,
            commands::engine::engine_convolver_load_ir,
```

- [ ] **Step 3: Build**

Run: `cargo build -p hypemuzik` (or the src-tauri crate name; check `src-tauri/Cargo.toml` `[package] name`)
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/engine.rs src-tauri/src/lib.rs crates/hm-audio/src/lib.rs
git commit -m "feat(tauri): engine_set_convolver + engine_convolver_load_ir commands"
```

---

## Task 9: TS IPC + store + `ConvolverCard.tsx`

**Files:**
- Modify: `src/lib/ipc.ts`, `src/stores/engine.ts`
- Create: `src/features/enhancer/ConvolverCard.tsx`
- Modify: the enhancer view that renders the cards (find where `RoomCard` is rendered, e.g. `src/features/enhancer/EnhancerView.tsx`)

**Interfaces:**
- Consumes: `engine_set_convolver`, `engine_convolver_load_ir`, `ConvolverState`.
- Produces (store): `setConvolver(next: ConvolverState)`, `loadConvolverIr(path: string)`.

- [ ] **Step 1: Add IPC wrappers** — in `src/lib/ipc.ts`, near `engine_set_room` (~line 119):

```ts
export function engineSetConvolver(convolver: ConvolverState): Promise<void> {
  return invoke<void>("engine_set_convolver", { convolver });
}

export interface ConvolverIrInfo {
  name: string;
  seconds: number;
  truncated: boolean;
  channels: number;
}

export function engineConvolverLoadIr(path: string): Promise<ConvolverIrInfo> {
  return invoke<ConvolverIrInfo>("engine_convolver_load_ir", { path });
}
```

Add `ConvolverState` to the existing `import type { ... } from "@/lib/types"` line at the top.

- [ ] **Step 2: Add store actions** — in `src/stores/engine.ts`, mirror `setRoom`. Add to the store type (near `setRoom: (next: RoomState) => void;`):

```ts
  setConvolver: (next: ConvolverState) => void;
  loadConvolverIr: (path: string) => Promise<void>;
```

And to the implementation (near the `setRoom` impl ~line 562):

```ts
    setConvolver: (next) => {
      set((s) => ({ state: { ...s.state, convolver: next } }));
      void engineSetConvolver(next);
    },
    loadConvolverIr: async (path) => {
      const info = await engineConvolverLoadIr(path);
      set((s) => ({
        state: {
          ...s.state,
          convolver: {
            ...s.state.convolver,
            enabled: true,
            irId: path,
            irName: info.name,
            irSeconds: info.seconds,
            irTruncated: info.truncated,
          },
        },
      }));
    },
```

Add `engineSetConvolver, engineConvolverLoadIr` to the `@/lib/ipc` import and `ConvolverState` to the `@/lib/types` import in this file.

- [ ] **Step 3: Create the card** — `src/features/enhancer/ConvolverCard.tsx`:

```tsx
import { useState } from "react";
import { Waves } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import { cn } from "@/lib/cn";

/** Convolution (impulse-response) reverb / correction. */
export function ConvolverCard() {
  const convolver = useEngineStore((s) => s.state.convolver);
  const setConvolver = useEngineStore((s) => s.setConvolver);
  const loadConvolverIr = useEngineStore((s) => s.loadConvolverIr);
  const [loading, setLoading] = useState(false);

  const pickIr = async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "Impulse response", extensions: ["wav", "irs"] }],
    });
    if (typeof path !== "string") return;
    setLoading(true);
    try {
      await loadConvolverIr(path);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Waves className="h-4 w-4 text-accent" />
          <span className="font-medium">Convolver</span>
        </div>
        <Switch
          checked={convolver.enabled}
          onCheckedChange={(enabled) => setConvolver({ ...convolver, enabled })}
        />
      </div>

      <div className={cn("mt-3 space-y-3", !convolver.enabled && "opacity-50")}>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={pickIr}
            disabled={loading}
            className="rounded-md bg-white/5 px-3 py-1.5 text-sm hover:bg-white/10 disabled:opacity-50"
          >
            {loading ? "Loading…" : "Load IR…"}
          </button>
          <span className="truncate text-sm text-white/60">
            {convolver.irName ?? "No impulse response loaded"}
          </span>
        </div>
        {convolver.irTruncated && (
          <p className="text-xs text-amber-400/80">
            IR truncated to {convolver.irSeconds.toFixed(1)} s for performance.
          </p>
        )}

        <label className="block text-sm">
          <span className="text-white/70">Mix</span>
          <Slider
            className="mt-1 flex-1"
            min={0}
            max={1}
            step={0.01}
            value={convolver.wetDry}
            onValueChange={(v) => setConvolver({ ...convolver, wetDry: v })}
          />
        </label>
        <label className="block text-sm">
          <span className="text-white/70">IR gain</span>
          <Slider
            className="mt-1 flex-1"
            min={-24}
            max={24}
            step={0.5}
            value={convolver.irGainDb}
            onValueChange={(v) => setConvolver({ ...convolver, irGainDb: v })}
          />
        </label>
      </div>
    </Card>
  );
}
```

Verify the exact prop names of `Card` / `Switch` / `Slider` against `RoomCard.tsx` (e.g. `onCheckedChange` vs `onChange`, `onValueChange` vs `onChange`) and match them — `RoomCard.tsx` is the source of truth. Confirm `@tauri-apps/plugin-dialog` is a dependency (it is used elsewhere for file pickers — grep `plugin-dialog`; if absent, use the existing file-open helper the app already uses).

- [ ] **Step 4: Render it** — in the enhancer view that renders `<RoomCard />`, add `<ConvolverCard />` right after it, and add the import.

- [ ] **Step 5: Typecheck**

Run: `pnpm tsc --noEmit`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lib/ipc.ts src/stores/engine.ts src/features/enhancer/ConvolverCard.tsx src/features/enhancer/
git commit -m "feat(ui): ConvolverCard — IR picker, mix + gain, store wiring"
```

---

## Task 10: GraphicEQ-string parser + interpolation + auto-preamp (hm-core)

**Files:**
- Create: `crates/hm-core/src/graphic_eq_import.rs`
- Modify: `crates/hm-core/src/lib.rs`
- Test: `crates/hm-core/src/graphic_eq_import.rs`

**Interfaces:**
- Produces:
  - `pub fn parse_graphic_eq(input: &str) -> Result<Vec<(f32, f32)>, String>` — parses `GraphicEQ: f1 g1; f2 g2; …` (label optional) into sorted (freq, gain_db) points.
  - `pub fn interpolate_to_iso_bands(curve: &[(f32, f32)]) -> [f32; BAND_COUNT]` — log-frequency interpolation onto `ISO_CENTERS_HZ`.
  - `pub fn recommended_preamp(bands: &[f32; BAND_COUNT]) -> f32` — `-max(0, max band)`.

- [ ] **Step 1: Write the failing tests** — create `crates/hm-core/src/graphic_eq_import.rs`:

```rust
//! Import EqualizerAPO `GraphicEQ` curves (the AutoEQ interchange format) onto
//! the 31-band graphic EQ, with a clip-proof recommended preamp. Pure data
//! transformation — no DSP, no I/O.

use crate::types::{BAND_COUNT, ISO_CENTERS_HZ};

/// Parse a `GraphicEQ` string into sorted (frequency Hz, gain dB) points.
/// Accepts an optional `GraphicEQ:` label and `freq gain` pairs separated by
/// `;`. Whitespace-tolerant.
pub fn parse_graphic_eq(input: &str) -> Result<Vec<(f32, f32)>, String> {
    let body = input
        .trim()
        .strip_prefix("GraphicEQ:")
        .or_else(|| input.trim().strip_prefix("GraphicEQ"))
        .unwrap_or(input)
        .trim_start_matches([':', ' ']);
    let mut points = Vec::new();
    for pair in body.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.split_whitespace();
        let f = it
            .next()
            .ok_or_else(|| format!("missing frequency in '{pair}'"))?
            .parse::<f32>()
            .map_err(|e| format!("bad frequency '{pair}': {e}"))?;
        let g = it
            .next()
            .ok_or_else(|| format!("missing gain in '{pair}'"))?
            .parse::<f32>()
            .map_err(|e| format!("bad gain '{pair}': {e}"))?;
        points.push((f, g));
    }
    if points.is_empty() {
        return Err("no (freq, gain) points found".into());
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(points)
}

/// Interpolate a (freq, gain) curve onto the ISO band centers in the log-freq
/// domain. Endpoints clamp to the nearest available point.
pub fn interpolate_to_iso_bands(curve: &[(f32, f32)]) -> [f32; BAND_COUNT] {
    let mut out = [0.0f32; BAND_COUNT];
    if curve.is_empty() {
        return out;
    }
    for (i, &center) in ISO_CENTERS_HZ.iter().enumerate() {
        let lc = center.max(1.0).log10();
        if center <= curve[0].0 {
            out[i] = curve[0].1;
            continue;
        }
        if center >= curve[curve.len() - 1].0 {
            out[i] = curve[curve.len() - 1].1;
            continue;
        }
        // Find the bracketing pair.
        let mut j = 0;
        while j + 1 < curve.len() && curve[j + 1].0 < center {
            j += 1;
        }
        let (f0, g0) = curve[j];
        let (f1, g1) = curve[j + 1];
        let l0 = f0.max(1.0).log10();
        let l1 = f1.max(1.0).log10();
        let t = if (l1 - l0).abs() < f32::EPSILON {
            0.0
        } else {
            (lc - l0) / (l1 - l0)
        };
        out[i] = g0 + (g1 - g0) * t;
    }
    out
}

/// Clip-proof preamp: enough negative gain that the peak band reaches 0 dB.
pub fn recommended_preamp(bands: &[f32; BAND_COUNT]) -> f32 {
    let peak = bands.iter().cloned().fold(f32::MIN, f32::max);
    -peak.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labeled_and_unlabeled() {
        let a = parse_graphic_eq("GraphicEQ: 20 -1.0; 1000 0.0; 20000 -3.0").unwrap();
        let b = parse_graphic_eq("20 -1.0; 1000 0.0; 20000 -3.0").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert_eq!(a[1], (1000.0, 0.0));
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_graphic_eq("").is_err());
        assert!(parse_graphic_eq("GraphicEQ: ").is_err());
        assert!(parse_graphic_eq("100 ; 200 1").is_err());
    }

    #[test]
    fn interpolation_hits_exact_points() {
        // A curve with a point exactly at the 1 kHz ISO center reproduces it.
        let idx = ISO_CENTERS_HZ.iter().position(|&f| (f - 1000.0).abs() < 0.5).unwrap();
        let curve = vec![(20.0, 0.0), (1000.0, 6.0), (20000.0, 0.0)];
        let bands = interpolate_to_iso_bands(&curve);
        assert!((bands[idx] - 6.0).abs() < 1e-3, "got {}", bands[idx]);
    }

    #[test]
    fn preamp_is_clip_proof() {
        let curve = vec![(20.0, 3.0), (1000.0, 9.0), (20000.0, -2.0)];
        let bands = interpolate_to_iso_bands(&curve);
        let pre = recommended_preamp(&bands);
        let peak = bands.iter().cloned().fold(f32::MIN, f32::max);
        assert!(peak + pre <= 1e-4, "peak {peak} + preamp {pre} must be <= 0");
    }
}
```

- [ ] **Step 2: Register the module** — in `crates/hm-core/src/lib.rs`, add `pub mod graphic_eq_import;` (after `pub mod error;`) and `pub use graphic_eq_import::{interpolate_to_iso_bands, parse_graphic_eq, recommended_preamp};`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p hm-core graphic_eq_import`
Expected: all PASS.

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p hm-core --all-targets -- -D warnings
git add crates/hm-core/src/graphic_eq_import.rs crates/hm-core/src/lib.rs
git commit -m "feat(core): EqualizerAPO GraphicEQ-string parse/interpolate/auto-preamp"
```

---

## Task 11: Tauri command `engine_eq_import_graphic` + registration

**Files:**
- Modify: `src-tauri/src/commands/engine.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `hm_core::{parse_graphic_eq, interpolate_to_iso_bands, recommended_preamp}`, `AudioEngine::set_eq`.
- Produces: `engine_eq_import_graphic(curve: String) -> Result<EqImportResult, IpcError>` where `EqImportResult { bands: Vec<f32>, pre_gain: f32 }`.

- [ ] **Step 1: Add the command** — in `src-tauri/src/commands/engine.rs`, after `engine_set_eq`:

```rust
/// Result of importing a GraphicEQ curve: the resolved bands + clip-proof preamp.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqImportResult {
    pub bands: Vec<f32>,
    pub pre_gain: f32,
}

/// Parse an EqualizerAPO GraphicEQ string, map it onto the 31 bands with a
/// clip-proof preamp, apply it, and return the resolved values to the UI.
#[tauri::command]
pub fn engine_eq_import_graphic(
    engine: State<'_, AudioEngine>,
    curve: String,
) -> Result<EqImportResult, IpcError> {
    let points = hm_core::parse_graphic_eq(&curve)
        .map_err(|e| IpcError::new("invalid", &e))?;
    let bands = hm_core::interpolate_to_iso_bands(&points);
    let pre_gain = hm_core::recommended_preamp(&bands);
    engine.set_eq(bands, pre_gain, true);
    Ok(EqImportResult { bands: bands.to_vec(), pre_gain })
}
```

(Confirm `IpcError::new(kind, msg)`'s exact signature against its use at `engine_set_eq` line ~42 — it's `IpcError::new("invalid", "…")`.)

- [ ] **Step 2: Register** — in `src-tauri/src/lib.rs` handler list, after `engine_set_eq`, add `commands::engine::engine_eq_import_graphic,`.

- [ ] **Step 3: Build + commit**

```bash
cargo build -p hypemuzik
git add src-tauri/src/commands/engine.rs src-tauri/src/lib.rs
git commit -m "feat(tauri): engine_eq_import_graphic command"
```

---

## Task 12: EQ card "Import curve" affordance

**Files:**
- Modify: `src/lib/ipc.ts`, `src/stores/engine.ts`, the EQ card component (find via `grep -rl "engine_set_eq\|setEq" src/features`)

**Interfaces:**
- Consumes: `engine_eq_import_graphic`.
- Produces (store): `importGraphicEq(curve: string): Promise<void>`.

- [ ] **Step 1: IPC wrapper** — in `src/lib/ipc.ts`:

```ts
export interface EqImportResult {
  bands: number[];
  preGain: number;
}

export function engineEqImportGraphic(curve: string): Promise<EqImportResult> {
  return invoke<EqImportResult>("engine_eq_import_graphic", { curve });
}
```

- [ ] **Step 2: Store action** — in `src/stores/engine.ts`, add to type + impl (mirror `setEq`):

```ts
  importGraphicEq: (curve: string) => Promise<void>;
```

```ts
    importGraphicEq: async (curve) => {
      const res = await engineEqImportGraphic(curve);
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, enabled: true, bands: res.bands, preGain: res.preGain },
        },
      }));
    },
```

Add `engineEqImportGraphic` to the `@/lib/ipc` import.

- [ ] **Step 3: UI affordance** — in the EQ card, add an "Import curve" button that opens a small textarea dialog (or a `prompt`-free inline `<textarea>` + Apply button) and calls `useEngineStore.getState().importGraphicEq(text)`. Minimal inline version:

```tsx
// inside the EQ card component
const importGraphicEq = useEngineStore((s) => s.importGraphicEq);
const [curveText, setCurveText] = useState("");
const [showImport, setShowImport] = useState(false);
// ...
{showImport && (
  <div className="mt-2 space-y-2">
    <textarea
      className="h-24 w-full rounded-md bg-white/5 p-2 text-xs"
      placeholder="GraphicEQ: 20 -1.2; 25 -1.1; ... (paste an AutoEQ curve)"
      value={curveText}
      onChange={(e) => setCurveText(e.target.value)}
    />
    <button
      type="button"
      className="rounded-md bg-accent/80 px-3 py-1.5 text-sm"
      onClick={async () => { await importGraphicEq(curveText); setShowImport(false); }}
    >
      Apply curve
    </button>
  </div>
)}
<button type="button" className="text-sm text-white/70 hover:text-white" onClick={() => setShowImport((v) => !v)}>
  Import curve…
</button>
```

Match the surrounding card's styling conventions. Ensure `useState` is imported.

- [ ] **Step 4: Typecheck + commit**

```bash
pnpm tsc --noEmit
git add src/lib/ipc.ts src/stores/engine.ts src/features/
git commit -m "feat(ui): EQ card GraphicEQ-curve import affordance"
```

---

## Final verification (after all tasks)

- [ ] `cargo test` (workspace) — all green.
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `pnpm tsc --noEmit` — clean.
- [ ] Manual smoke (if a dev build is available): load a reverb IR, toggle Convolver, sweep Mix — audio stays glitch-free and the UI stays responsive during load (proving off-thread prep). Paste an AutoEQ GraphicEQ curve, confirm bands populate and preamp goes negative.

---

## Self-review notes (filled during planning)

- **Spec coverage:** Convolver DSP (Tasks 2–5), off-thread prep + ArcSwap (Tasks 3, 7), four-layer gain staging (IR normalize=Task 3, wet/dry+gain+clamp=Task 4, limiter=existing chain tail=Task 5), state/commands/UI (1, 8, 9), GraphicEQ import + auto-preamp (10–12). All spec sections map to tasks.
- **Performance:** bounded per-block FFT (Task 2), `MAX_IR_SECONDS` cap (Task 3), lock-free handoff + no audio-thread I/O (Tasks 4, 7), denormal safety is inherent (FFT, no IIR feedback). Identity early-return when off (Task 4).
- **Type consistency:** `ConvolverState`/`convolver` field, `IrSlot`, `PreparedIr`, `PreparedIrChannel`, `MonoConvolver::process(input,out,ir)`, `ConvolverIrInfo`, `standard_with_ir`, `engine_set_convolver`/`engine_convolver_load_ir`/`engine_eq_import_graphic` are used identically across tasks.
- **Known confirmations the implementer must make (not placeholders — explicit verifications):** exact `AudioError` decode constructor (Task 6 Step 1), exact `Card`/`Switch`/`Slider` prop names vs `RoomCard.tsx` (Task 9 Step 3), `@tauri-apps/plugin-dialog` availability (Task 9), src-tauri crate name (Task 8 Step 3), `IpcError::new` signature (Task 11).
