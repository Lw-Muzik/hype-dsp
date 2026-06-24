//! Real-time-safe opcode evaluator for the LiveProg EEL2-subset VM.
//!
//! # Design constraints
//!
//! * **Allocation-free hot path**: `run_sample` and `run_init` never allocate.
//!   The operand stack is a fixed `[f32; STACK_CAP]` array on the call stack;
//!   the register file is the caller's `&mut [f32]` slice.
//!
//! * **Instruction budget**: every executed op decrements a `budget` counter.
//!   When the budget reaches zero the run aborts immediately (returns), making
//!   any user-written infinite loop (e.g. `while(1)(...)`) audio-thread-safe.
//!
//! * **NaN / Inf guard**: non-finite results from arithmetic or builtins are
//!   replaced with `0.0` before being pushed onto the stack.
//!
//! * **No panics**: all array accesses are bounds-checked; stack overflow and
//!   underflow are handled gracefully (overflow → drop, underflow → 0.0).

use super::compiler::{Builtin, Op, Program};

/// Operand-stack capacity.  A valid compile will never exceed this depth,
/// but we guard defensively anyway.
const STACK_CAP: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Internal stack type (zero-heap, fixed array)
// ─────────────────────────────────────────────────────────────────────────────

struct Stack {
    data: [f32; STACK_CAP],
    top: usize,
}

impl Stack {
    #[inline(always)]
    fn new() -> Self {
        Stack {
            data: [0.0f32; STACK_CAP],
            top: 0,
        }
    }

    /// Push a value.  If the stack is full, the value is silently dropped.
    #[inline(always)]
    fn push(&mut self, v: f32) {
        if self.top < STACK_CAP {
            self.data[self.top] = v;
            self.top += 1;
        }
        // overflow → drop (no panic, no OOB)
    }

    /// Pop a value.  Returns `0.0` on underflow.
    #[inline(always)]
    fn pop(&mut self) -> f32 {
        if self.top == 0 {
            return 0.0;
        }
        self.top -= 1;
        self.data[self.top]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NaN / Inf guard
// ─────────────────────────────────────────────────────────────────────────────

/// Replace non-finite values with `0.0`.
#[inline(always)]
fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// True when a condition value should be treated as *false*:
/// exactly `0.0` or any non-finite value.
#[inline(always)]
fn is_false(v: f32) -> bool {
    v == 0.0 || !v.is_finite()
}

// ─────────────────────────────────────────────────────────────────────────────
// Core execution loop
// ─────────────────────────────────────────────────────────────────────────────

/// Execute `ops` with the given register file and instruction budget.
///
/// Returns when: (a) all ops are exhausted, or (b) budget reaches zero.
/// Never panics.  Never allocates.
fn execute(ops: &[Op], regs: &mut [f32], budget: &mut u32) {
    let mut stack = Stack::new();
    let mut pc: usize = 0;
    let len = ops.len();

    while pc < len {
        // Budget check — abort on exhaustion (RT-safety guarantee).
        if *budget == 0 {
            return;
        }
        *budget -= 1;

        // SAFETY: pc < len is the loop invariant checked above.
        let op = unsafe { ops.get_unchecked(pc) };
        pc += 1;

        match op {
            // ── Stack / register ops ──────────────────────────────────────
            Op::PushConst(v) => {
                stack.push(*v);
            }
            Op::LoadReg(i) => {
                let v = regs.get(*i as usize).copied().unwrap_or(0.0);
                stack.push(v);
            }
            Op::StoreReg(i) => {
                let v = stack.pop();
                if let Some(slot) = regs.get_mut(*i as usize) {
                    *slot = v;
                }
                // OOB store → no-op (never panic)
            }
            Op::Pop => {
                stack.pop();
            }

            // ── Binary arithmetic ─────────────────────────────────────────
            Op::Add => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(finite_or_zero(a + b));
            }
            Op::Sub => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(finite_or_zero(a - b));
            }
            Op::Mul => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(finite_or_zero(a * b));
            }
            Op::Div => {
                let b = stack.pop();
                let a = stack.pop();
                // Division by zero → non-finite → guarded to 0.0
                stack.push(finite_or_zero(a / b));
            }
            Op::Mod => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(finite_or_zero(a % b));
            }
            Op::Pow => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(finite_or_zero(a.powf(b)));
            }

            // ── Unary arithmetic ──────────────────────────────────────────
            Op::Neg => {
                let a = stack.pop();
                stack.push(finite_or_zero(-a));
            }

            // ── Comparison / logic ────────────────────────────────────────
            Op::Eq => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if (a - b).abs() < f32::EPSILON { 1.0 } else { 0.0 });
            }
            Op::Ne => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if (a - b).abs() >= f32::EPSILON { 1.0 } else { 0.0 });
            }
            Op::Lt => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if a < b { 1.0 } else { 0.0 });
            }
            Op::Le => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if a <= b { 1.0 } else { 0.0 });
            }
            Op::Gt => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if a > b { 1.0 } else { 0.0 });
            }
            Op::Ge => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if a >= b { 1.0 } else { 0.0 });
            }
            Op::And => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if !is_false(a) && !is_false(b) { 1.0 } else { 0.0 });
            }
            Op::Or => {
                let b = stack.pop();
                let a = stack.pop();
                stack.push(if !is_false(a) || !is_false(b) { 1.0 } else { 0.0 });
            }
            Op::Not => {
                let a = stack.pop();
                stack.push(if is_false(a) { 1.0 } else { 0.0 });
            }

            // ── Builtin function call ─────────────────────────────────────
            Op::Call(builtin) => {
                let result = dispatch_builtin(*builtin, &mut stack);
                stack.push(finite_or_zero(result));
            }

            // ── Control flow ──────────────────────────────────────────────
            Op::JumpIfFalse(target) => {
                let cond = stack.pop();
                if is_false(cond) {
                    let t = *target as usize;
                    if t < len {
                        pc = t;
                    } else {
                        // Out-of-bounds target → abort (safety)
                        return;
                    }
                }
            }
            Op::Jump(target) => {
                let t = *target as usize;
                if t < len {
                    pc = t;
                } else {
                    // Out-of-bounds target → abort (safety)
                    return;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Builtin dispatch
// ─────────────────────────────────────────────────────────────────────────────

/// Pop the right number of arguments and call the builtin math function.
///
/// For 2-arg builtins, pop `b` (top of stack = right arg) first, then `a`
/// (left arg), and call `f(a, b)` — matching the compiler's push order.
#[inline(always)]
fn dispatch_builtin(b: Builtin, stack: &mut Stack) -> f32 {
    match b {
        // ── 1-arg builtins ────────────────────────────────────────────────
        Builtin::Sin => {
            let a = stack.pop();
            a.sin()
        }
        Builtin::Cos => {
            let a = stack.pop();
            a.cos()
        }
        Builtin::Tan => {
            let a = stack.pop();
            a.tan()
        }
        Builtin::Asin => {
            let a = stack.pop();
            a.asin()
        }
        Builtin::Acos => {
            let a = stack.pop();
            a.acos()
        }
        Builtin::Atan => {
            let a = stack.pop();
            a.atan()
        }
        Builtin::Sqrt => {
            let a = stack.pop();
            a.sqrt()
        }
        Builtin::Exp => {
            let a = stack.pop();
            a.exp()
        }
        Builtin::Log => {
            let a = stack.pop();
            a.ln()
        }
        Builtin::Log10 => {
            let a = stack.pop();
            a.log10()
        }
        Builtin::Abs => {
            let a = stack.pop();
            a.abs()
        }
        Builtin::Floor => {
            let a = stack.pop();
            a.floor()
        }
        Builtin::Ceil => {
            let a = stack.pop();
            a.ceil()
        }
        Builtin::Round => {
            let a = stack.pop();
            a.round()
        }
        Builtin::Sign => {
            let a = stack.pop();
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Builtin::Tanh => {
            let a = stack.pop();
            a.tanh()
        }

        // ── 2-arg builtins ────────────────────────────────────────────────
        // Pop b (top of stack = right/second arg) then a (left/first arg).
        Builtin::Atan2 => {
            let b = stack.pop();
            let a = stack.pop();
            a.atan2(b)
        }
        Builtin::Pow => {
            let b = stack.pop();
            let a = stack.pop();
            a.powf(b)
        }
        Builtin::Min => {
            let b = stack.pop();
            let a = stack.pop();
            a.min(b)
        }
        Builtin::Max => {
            let b = stack.pop();
            let a = stack.pop();
            a.max(b)
        }
        Builtin::Fmod => {
            let b = stack.pop();
            let a = stack.pop();
            a % b
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Run the `@init` section of `prog` once.
///
/// Initialises user registers.  `regs` must be pre-sized to at least
/// `prog.num_regs` (the caller is responsible; this function never allocates).
/// `budget` bounds runaway init code; passing `0` is a no-op.
pub fn run_init(prog: &Program, regs: &mut [f32], budget: u32) {
    let mut remaining = budget;
    execute(&prog.init, regs, &mut remaining);
}

/// Run the `@sample` section of `prog` for one stereo frame.
///
/// 1. Writes `spl[0]` → `regs[spl0_reg]`, `spl[1]` → `regs[spl1_reg]`.
/// 2. Executes `prog.sample` ops (budget-bounded, allocation-free).
/// 3. Reads `regs[spl0_reg]` → `spl[0]`, `regs[spl1_reg]` → `spl[1]`.
///    Non-finite output samples are replaced with `0.0` before writing back.
///
/// `regs` must be pre-sized to at least `prog.num_regs`.
pub fn run_sample(prog: &Program, regs: &mut [f32], spl: &mut [f32; 2], budget: u32) {
    // Write input samples into the register file.
    if let Some(r) = regs.get_mut(prog.spl0_reg as usize) {
        *r = spl[0];
    }
    if let Some(r) = regs.get_mut(prog.spl1_reg as usize) {
        *r = spl[1];
    }

    // Execute the @sample ops.
    let mut remaining = budget;
    execute(&prog.sample, regs, &mut remaining);

    // Read output samples back, guarding against non-finite values.
    spl[0] = regs
        .get(prog.spl0_reg as usize)
        .copied()
        .map(finite_or_zero)
        .unwrap_or(0.0);
    spl[1] = regs
        .get(prog.spl1_reg as usize)
        .copied()
        .map(finite_or_zero)
        .unwrap_or(0.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::compile;

    /// Allocate a register file sized for `prog`.
    fn make_regs(prog: &Program) -> Vec<f32> {
        vec![0.0f32; prog.num_regs]
    }

    // ── 1. gain_halves ───────────────────────────────────────────────────────

    #[test]
    fn gain_halves() {
        let prog = compile("@sample spl0=spl0*0.5; spl1=spl1*0.5;").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [1.0f32, 0.8f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!(
            (spl[0] - 0.5).abs() < 1e-6,
            "spl0 should be 0.5, got {}",
            spl[0]
        );
        assert!(
            (spl[1] - 0.4).abs() < 1e-6,
            "spl1 should be 0.4, got {}",
            spl[1]
        );
    }

    // ── 2. init_runs_once ────────────────────────────────────────────────────

    #[test]
    fn init_runs_once() {
        let prog = compile("@init n=0; @sample n=n+1; spl0=n;").expect("compile");
        let mut regs = make_regs(&prog);

        // Run init exactly once.
        run_init(&prog, &mut regs, 10_000);

        // Run K sample frames.
        const K: usize = 7;
        for _ in 0..K {
            let mut spl = [0.0f32; 2];
            run_sample(&prog, &mut regs, &mut spl, 10_000);
        }

        // n should equal K (persisted in regs, init not re-run).
        let n_val = regs[prog.num_regs - 1]; // 'n' is the last allocated reg
        // More robustly: read via spl0 from the last run.
        // We already ran K times so read from regs directly.
        // The 'n' register is whatever reg the compiler assigned; easiest is
        // to run one more sample and check spl0.
        let mut spl = [0.0f32; 2];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        // After K+1 total samples, spl0 == K+1
        let _ = n_val; // unused
        assert!(
            (spl[0] - (K as f32 + 1.0)).abs() < 1e-5,
            "expected spl0 == {}, got {}",
            K + 1,
            spl[0]
        );
    }

    // ── 3. precedence_eval ───────────────────────────────────────────────────

    #[test]
    fn precedence_eval() {
        let prog = compile("spl0 = 1+2*3;").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [0.0f32; 2];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!(
            (spl[0] - 7.0).abs() < 1e-5,
            "expected spl0 == 7, got {}",
            spl[0]
        );
    }

    // ── 4. builtins ──────────────────────────────────────────────────────────

    #[test]
    fn builtins_sqrt() {
        let prog = compile("spl0 = sqrt(spl0);").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [4.0f32, 0.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!((spl[0] - 2.0).abs() < 1e-5, "sqrt(4) = {}", spl[0]);
    }

    #[test]
    fn builtins_sin() {
        let prog = compile("spl0 = sin(spl0);").expect("compile");
        let mut regs = make_regs(&prog);
        let input = std::f32::consts::FRAC_PI_2;
        let mut spl = [input, 0.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!((spl[0] - 1.0).abs() < 1e-5, "sin(pi/2) = {}", spl[0]);
    }

    #[test]
    fn builtins_abs() {
        let prog = compile("spl0 = abs(spl0);").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [-3.5f32, 0.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!((spl[0] - 3.5).abs() < 1e-5, "abs(-3.5) = {}", spl[0]);
    }

    #[test]
    fn builtins_min_max() {
        let prog_min = compile("spl0 = min(spl0, spl1);").expect("compile min");
        let prog_max = compile("spl0 = max(spl0, spl1);").expect("compile max");

        let mut regs = make_regs(&prog_min);
        let mut spl = [3.0f32, 5.0f32];
        run_sample(&prog_min, &mut regs, &mut spl, 10_000);
        assert!((spl[0] - 3.0).abs() < 1e-5, "min(3,5)={}", spl[0]);

        let mut regs = make_regs(&prog_max);
        let mut spl = [3.0f32, 5.0f32];
        run_sample(&prog_max, &mut regs, &mut spl, 10_000);
        assert!((spl[0] - 5.0).abs() < 1e-5, "max(3,5)={}", spl[0]);
    }

    #[test]
    fn builtins_atan2() {
        // atan2(y=1, x=1) == pi/4
        let prog = compile("spl0 = atan2(spl0, spl1);").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [1.0f32, 1.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        let expected = std::f32::consts::FRAC_PI_4;
        assert!(
            (spl[0] - expected).abs() < 1e-5,
            "atan2(1,1)={}, expected {}",
            spl[0],
            expected
        );
    }

    #[test]
    fn builtins_tanh() {
        let prog = compile("spl0 = tanh(spl0);").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [1.0f32, 0.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        let expected = 1.0f32.tanh();
        assert!(
            (spl[0] - expected).abs() < 1e-5,
            "tanh(1)={}, expected {}",
            spl[0],
            expected
        );
    }

    // ── 5. RUNAWAY LOOP (critical RT-safety gate) ─────────────────────────────
    //
    // The test completing proves the budget terminated the loop.
    // If the budget mechanism were broken this test would hang indefinitely.

    #[test]
    fn runaway_loop_budget_terminates() {
        let prog = compile("@sample while(1) ( n=n+1; );").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [0.0f32; 2];

        // Small budget so the test completes in microseconds.
        run_sample(&prog, &mut regs, &mut spl, 500);

        // The test reaching this line proves run_sample returned.
        assert!(spl[0].is_finite(), "spl0 must be finite after runaway loop");
        assert!(spl[1].is_finite(), "spl1 must be finite after runaway loop");
    }

    // ── 6. NaN guard ─────────────────────────────────────────────────────────

    #[test]
    fn nan_guard_div_zero() {
        // 0.0 / 0.0 = NaN → should become 0.0
        let prog = compile("spl0 = spl0 / spl1;").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [0.0f32, 0.0f32];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!(
            spl[0].is_finite(),
            "0/0 must produce finite output, got {}",
            spl[0]
        );
        assert_eq!(spl[0], 0.0, "0/0 should be guarded to 0.0");
    }

    #[test]
    fn nan_guard_sqrt_negative() {
        // sqrt(-1) = NaN → should become 0.0
        let prog = compile("spl0 = sqrt(0 - 1);").expect("compile");
        let mut regs = make_regs(&prog);
        let mut spl = [0.0f32; 2];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!(
            spl[0].is_finite(),
            "sqrt(-1) must be finite, got {}",
            spl[0]
        );
        assert_eq!(spl[0], 0.0, "sqrt(-1) should be guarded to 0.0");
    }

    // ── 7. loop_counted ──────────────────────────────────────────────────────

    #[test]
    fn loop_counted() {
        let prog =
            compile("@init s=0; @sample s=0; loop(5) ( s=s+1; ); spl0=s;").expect("compile");
        let mut regs = make_regs(&prog);
        run_init(&prog, &mut regs, 10_000);
        let mut spl = [0.0f32; 2];
        run_sample(&prog, &mut regs, &mut spl, 10_000);
        assert!(
            (spl[0] - 5.0).abs() < 1e-5,
            "loop(5) should yield s==5, got {}",
            spl[0]
        );
    }
}
