//! LiveProg script processing stage — runs a compiled EEL2-subset [`Program`]
//! per audio frame in the DSP chain.
//!
//! # Design
//!
//! * **Lock-free slot**: the compiled [`Program`] is published by the command
//!   thread via an [`ArcSwap`] (`ScriptSlot`), mirroring the convolver's
//!   `IrSlot` pattern.  The audio thread calls `load()` once per block — no
//!   mutex, no allocation on the steady path.
//!
//! * **Program-change detection**: the processor stores the last-seen `Arc`
//!   (via `Arc::ptr_eq`) so it can detect when a new program has been published.
//!   On change it resizes the register file and runs `run_init` once — the only
//!   moment reallocation is allowed.
//!
//! * **RT-safety**: once a program is stable the steady path is
//!   `load() + run_sample()` — no allocation, no locks, no I/O.
//!
//! * **Output bounding**: after running the VM, each sample is
//!   flush-denormaled and clamped to `[-4.0, 4.0]` (the master limiter
//!   downstream provides the final ceiling).

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::{AudioProcessor, ProcessorParams};
use crate::script::{run_init, run_sample_within, Program};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// A lock-free, shared handle to the active compiled [`Program`].
///
/// `None` means no script is loaded (identity pass-through).
/// The command thread `store`s a new `Arc<Program>`; the audio thread `load`s it.
pub type ScriptSlot = Arc<ArcSwap<Option<Arc<Program>>>>;

/// Create an empty script slot (no program loaded).
pub fn empty_script_slot() -> ScriptSlot {
    Arc::new(ArcSwap::from_pointee(None))
}

// ─────────────────────────────────────────────────────────────────────────────
// ScriptProcessor
// ─────────────────────────────────────────────────────────────────────────────

/// Total instruction budget for one audio callback (one `process()` call).
///
/// This budget is shared across ALL frames in the block — once exhausted,
/// remaining frames pass through unchanged (identity), keeping per-callback
/// CPU cost strictly bounded regardless of buffer size.
///
/// Chosen rationale:
///   - Typical 512-frame block at 48 kHz ≈ 10.7 ms of wall time.
///   - A real EEL2 script runs O(10–50) ops/frame → well under 50k ops/block.
///   - 2_000_000 allows ~3 900 ops/frame for a 512-frame block — generous for
///     legitimate scripts while capping a `while(1)` at a few µs per callback.
const BLOCK_BUDGET: u32 = 2_000_000;

/// LiveProg DSP stage — runs a compiled EEL2-subset script per audio frame.
pub struct ScriptProcessor {
    /// Lock-free handle to the published program (shared with command thread).
    slot: ScriptSlot,
    /// Pre-sized register file for the currently-loaded program.
    regs: Vec<f32>,
    /// Last-seen `Arc<Program>` for pointer-equality change detection.
    /// `None` when no program has ever been loaded.
    last_prog: Option<Arc<Program>>,
    /// Sample rate, written to `srate_reg` when a new program is loaded.
    sample_rate: f32,
    /// Whether the stage is active; `false` → identity pass-through.
    enabled: bool,
}

impl ScriptProcessor {
    /// Create a stage with a fresh, empty slot.
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self::with_slot(sample_rate, channels, empty_script_slot())
    }

    /// Create a stage wired to an externally-owned slot so the engine can
    /// publish compiled programs to this stage.
    pub fn with_slot(sample_rate: f32, _channels: usize, slot: ScriptSlot) -> Self {
        Self {
            slot,
            regs: Vec::new(),
            last_prog: None,
            sample_rate,
            enabled: false,
        }
    }

    /// Expose the slot so the engine can clone it and publish programs.
    pub fn slot(&self) -> ScriptSlot {
        self.slot.clone()
    }
}

/// Flush near-denormal values to zero to prevent CPU slowdowns in the
/// VM feedback paths (mirrors the pattern used in `room.rs`).
#[inline(always)]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

impl AudioProcessor for ScriptProcessor {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        self.sample_rate = sample_rate;
        // If a program is already loaded, update its srate register immediately.
        if let Some(ref prog) = self.last_prog {
            if let Some(r) = self.regs.get_mut(prog.srate_reg as usize) {
                *r = sample_rate;
            }
        }
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        // Identity: disabled or nothing to process.
        if !self.enabled || channels == 0 || buffer.is_empty() {
            return;
        }

        // Load the current program snapshot — single atomic load, no allocation.
        let guard = self.slot.load();
        let prog_opt: &Option<Arc<Program>> = &guard;
        let prog = match prog_opt {
            Some(p) => p,
            None => return, // no program → identity
        };

        // Detect program change by Arc pointer equality.
        let changed = match &self.last_prog {
            None => true,
            Some(prev) => !Arc::ptr_eq(prev, prog),
        };

        if changed {
            // Resize the register file and zero-fill.  This is the ONLY place
            // an allocation can happen; it is off the steady path.
            self.regs.clear();
            self.regs.resize(prog.num_regs, 0.0);
            // Set the sample-rate register before init so scripts can use it.
            if let Some(r) = self.regs.get_mut(prog.srate_reg as usize) {
                *r = self.sample_rate;
            }
            // Run @init once to let the script initialise its state.
            // Uses its own fixed budget (off the hot path; allocation already happened).
            run_init(prog, &mut self.regs, 1_000_000);
            // Remember this program so we can detect the next change.
            self.last_prog = Some(prog.clone());
        }

        // Per-frame processing — allocation-free steady path.
        //
        // One shared budget for the ENTIRE block: once exhausted, subsequent
        // run_sample calls return immediately (identity for those frames).
        // This caps total per-callback CPU cost regardless of buffer size.
        let frames = buffer.len() / channels;
        let stereo = channels >= 2;
        let mut budget: u32 = BLOCK_BUDGET;
        for f in 0..frames {
            let l = buffer[f * channels];
            let r = if stereo { buffer[f * channels + 1] } else { l };

            let mut spl = [l, r];
            run_sample_within(prog, &mut self.regs, &mut spl, &mut budget);

            // Flush denormals and clamp to ±4.0.
            let out_l = flush(spl[0]).clamp(-4.0, 4.0);
            let out_r = flush(spl[1]).clamp(-4.0, 4.0);

            buffer[f * channels] = out_l;
            if stereo {
                buffer[f * channels + 1] = out_r;
            }
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.script.enabled;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::EngineState;
    use crate::script::compile;

    /// Build a `ProcessorParams` (= `EngineState`) with script toggled.
    fn params_with_script(enabled: bool) -> EngineState {
        EngineState {
            script: hm_core::ScriptState { enabled, ..Default::default() },
            ..Default::default()
        }
    }

    // ── 1. disabled_is_identity ──────────────────────────────────────────────
    //
    // When the stage is disabled (default), the buffer must be bit-exact.

    #[test]
    fn disabled_is_identity() {
        let mut proc = ScriptProcessor::new(48_000.0, 2);
        // Default state: enabled=false, no program.
        proc.set_params(&EngineState::default());

        let original: Vec<f32> = (0..16).map(|i| i as f32 * 0.1 - 0.75).collect();
        let mut buf = original.clone();
        proc.process(&mut buf, 2);
        assert_eq!(buf, original, "disabled ScriptProcessor must not alter the buffer");
    }

    // ── 2. no_program_is_identity ────────────────────────────────────────────
    //
    // Enabled but slot holds None → identity.

    #[test]
    fn no_program_is_identity() {
        let mut proc = ScriptProcessor::new(48_000.0, 2);
        proc.set_params(&params_with_script(true)); // enabled but no program

        let original: Vec<f32> = (0..16).map(|i| (i as f32 * 0.05).sin()).collect();
        let mut buf = original.clone();
        proc.process(&mut buf, 2);
        assert_eq!(buf, original, "enabled ScriptProcessor without a program must be identity");
    }

    // ── 3. gain_program_halves ───────────────────────────────────────────────
    //
    // Load `spl0=spl0*0.5; spl1=spl1*0.5;` into the slot, enable → halves.

    #[test]
    fn gain_program_halves() {
        let slot = empty_script_slot();
        let mut proc = ScriptProcessor::with_slot(48_000.0, 2, slot.clone());
        proc.set_params(&params_with_script(true));

        // Compile and publish the half-gain program.
        let prog = compile("spl0=spl0*0.5; spl1=spl1*0.5;").expect("compile gain program");
        slot.store(Arc::new(Some(Arc::new(prog))));

        // Stereo buffer: 4 frames × 2 channels.
        let mut buf = vec![1.0f32, 0.8, 0.6, -0.4, -0.2, 0.3, 0.9, -0.7];
        let expected: Vec<f32> = buf.iter().map(|&x| x * 0.5).collect();
        proc.process(&mut buf, 2);

        for (i, (&got, &want)) in buf.iter().zip(expected.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-5,
                "sample[{}]: got {}, expected {}",
                i, got, want
            );
        }
    }

    // ── 4. stays_bounded ─────────────────────────────────────────────────────
    //
    // A program that multiplies by a huge number → output clamped to ±4.0.

    #[test]
    fn stays_bounded() {
        let slot = empty_script_slot();
        let mut proc = ScriptProcessor::with_slot(48_000.0, 2, slot.clone());
        proc.set_params(&params_with_script(true));

        // Multiply by a large constant — output would be astronomical.
        let prog = compile("spl0=spl0*100000.0; spl1=spl1*100000.0;")
            .expect("compile loud program");
        slot.store(Arc::new(Some(Arc::new(prog))));

        let mut buf: Vec<f32> = (0..32).map(|i| if i % 2 == 0 { 0.9f32 } else { -0.9f32 }).collect();
        proc.process(&mut buf, 2);

        for &s in &buf {
            assert!(
                s.abs() <= 4.0,
                "output must be clamped to ±4.0, got {}",
                s
            );
        }
    }

    // ── 5. program_change_detected ───────────────────────────────────────────
    //
    // Swap from one program to another; the second one must take effect.

    #[test]
    fn program_change_detected() {
        let slot = empty_script_slot();
        let mut proc = ScriptProcessor::with_slot(48_000.0, 2, slot.clone());
        proc.set_params(&params_with_script(true));

        // First program: multiply by 2.
        let prog1 = compile("spl0=spl0*2.0; spl1=spl1*2.0;").expect("compile prog1");
        slot.store(Arc::new(Some(Arc::new(prog1))));

        let mut buf = vec![0.5f32, -0.5f32];
        proc.process(&mut buf, 2);
        // After first program: 0.5 * 2 = 1.0, -0.5 * 2 = -1.0
        assert!((buf[0] - 1.0).abs() < 1e-5, "prog1 l: got {}", buf[0]);
        assert!((buf[1] - (-1.0)).abs() < 1e-5, "prog1 r: got {}", buf[1]);

        // Second program: multiply by 0.25.
        let prog2 = compile("spl0=spl0*0.25; spl1=spl1*0.25;").expect("compile prog2");
        slot.store(Arc::new(Some(Arc::new(prog2))));

        let mut buf2 = vec![0.8f32, -0.8f32];
        proc.process(&mut buf2, 2);
        // After second program: 0.8 * 0.25 = 0.2, -0.8 * 0.25 = -0.2
        assert!((buf2[0] - 0.2).abs() < 1e-5, "prog2 l: got {}", buf2[0]);
        assert!((buf2[1] - (-0.2)).abs() < 1e-5, "prog2 r: got {}", buf2[1]);
    }
}
