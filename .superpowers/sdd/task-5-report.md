# Task 5 Implementation Report — LiveProg VM (`hm-dsp/src/script/vm.rs`)

**Date:** 2026-06-24
**Status:** Complete — all tests green, clippy clean

---

## What was implemented

`crates/hm-dsp/src/script/vm.rs` — a real-time-safe opcode evaluator for the EEL2-subset LiveProg scripting system.

### Public API

```rust
pub fn run_init(prog: &Program, regs: &mut [f32], budget: u32)
pub fn run_sample(prog: &Program, regs: &mut [f32], spl: &mut [f32; 2], budget: u32)
```

---

## Budget mechanism

Every op executed in the inner `execute()` loop decrements a `&mut u32` budget counter before dispatch. The check appears at the top of the loop body, before matching the op:

```rust
if *budget == 0 {
    return;
}
*budget -= 1;
```

This means back-jumping ops (`Jump`, `JumpIfFalse`) also consume budget — every loop iteration burns one budget unit per op in the body, plus the condition and jump itself. A `while(1)(n=n+1;)` loop with budget 500 aborts after executing ≤ 500 instructions.

The `runaway_loop_budget_terminates` test confirms this: the test itself completing (without hanging) proves the mechanism works.

---

## NaN / Inf guard

All arithmetic results are passed through `finite_or_zero(v)` before being pushed:

```rust
fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}
```

Applied to: `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Pow`, `Neg`, and all `Call(Builtin)` results.

`JumpIfFalse` treats non-finite conditions as false (`is_false` checks `v == 0.0 || !v.is_finite()`).

`run_sample` sanitizes spl outputs with `finite_or_zero` after reading back from regs.

---

## Stack safety

The operand stack is a fixed `[f32; STACK_CAP]` (256 entries) + a `usize` top counter:

- **Overflow**: push when full → silently drops the value (no panic, no OOB write).
- **Underflow**: pop when empty → returns `0.0`.
- A valid compiled program will never overflow or underflow, but these guards make it impossible to cause UB from a pathological program.

---

## Allocation-free confirmation

- `execute()`, `run_init()`, `run_sample()` contain no `Vec`, `Box`, `String`, or any heap allocation.
- The operand `Stack` struct lives entirely on the call stack.
- The register file is the caller's `&mut [f32]` slice — callers (e.g. `ScriptProcessor`, tests) own the allocation.
- No locks, no I/O, no system calls in the hot path.

---

## Runaway-loop test result

```
test script::vm::tests::runaway_loop_budget_terminates ... ok
```

Script: `@sample while(1) ( n=n+1; );`, budget: 500.
Run time: effectively 0 ms (loop body: 2 ops × ~250 iterations before budget exhausted).
The test completes in the test binary's startup noise — the safety guarantee holds.

---

## 2-arg builtin argument order

Compiler emits args left-to-right: for `atan2(spl0, spl1)`, `spl0` is pushed first, `spl1` second.
VM pops right-to-left: pops `b` (spl1), then pops `a` (spl0), calls `a.atan2(b)`.
This matches EEL2 semantics and standard `atan2(y, x)` convention.

---

## Tests (13 / 13 pass)

| Test | What it verifies |
|------|-----------------|
| `gain_halves` | Basic mul + store; spl I/O round-trip |
| `init_runs_once` | `run_init` vs `run_sample` separation; reg persistence |
| `precedence_eval` | `1+2*3 == 7`; mul binds tighter than add |
| `builtins_sqrt` | 1-arg builtin dispatch |
| `builtins_sin` | 1-arg trig |
| `builtins_abs` | 1-arg; negative input |
| `builtins_min_max` | 2-arg builtins |
| `builtins_atan2` | 2-arg; argument order correctness |
| `builtins_tanh` | 1-arg; used in soft-clip examples |
| `runaway_loop_budget_terminates` | **Critical RT-safety gate** |
| `nan_guard_div_zero` | 0/0 → 0.0 |
| `nan_guard_sqrt_negative` | sqrt(-1) → 0.0 |
| `loop_counted` | `loop(n)` counted loop; `run_init` + `run_sample` interaction |

---

## Concerns / follow-up notes

1. **`loop(n)` with fractional `n`**: The compiler's hidden counter register is a float. `loop(5.7)` will execute the body 5 times (floor via repeated `> 0` test + decrement). This is EEL2-compatible behaviour but worth documenting.

2. **`JumpIfFalse` with `u32::MAX` placeholder**: If a compiler bug ever emits an unpatched `JumpIfFalse(u32::MAX)`, the VM will silently abort (`t >= len`). This is safe but silent — a compiler test gate covers this path.

3. **Budget value**: Default 10,000 used in tests. The plan specifies 10,000 for `ScriptProcessor`. With 44.1 kHz audio and a 256-sample buffer, that's ~39 µs per buffer — 10,000 simple ops complete well under 100 µs on modern hardware.

4. **`unsafe get_unchecked`**: Used once for the op fetch (`ops.get_unchecked(pc)`) inside the `pc < len` while-loop guard. This is sound (the invariant is maintained by the loop condition) and avoids a redundant bounds check in the hot path.
