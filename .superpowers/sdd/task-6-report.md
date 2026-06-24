# Task 6 Report: ScriptProcessor Stage + ArcSwap Slot + Chain Insert

## ScriptSlot type definition

```rust
pub type ScriptSlot = Arc<ArcSwap<Option<Arc<Program>>>>;
```

Mirrors the `IrSlot` pattern from `convolver.rs` exactly. The outer `Arc` enables
the slot to be shared (cloned cheaply) between the engine and the `ScriptProcessor`.
The inner `ArcSwap<Option<Arc<Program>>>` provides lock-free, wait-free publish from
the command thread and load from the audio thread. `Option` encodes the "no program"
case; `None` → identity pass-through.

`empty_script_slot()` creates the slot holding `Arc::new(None)`.

## Program-change detection mechanism

`ScriptProcessor` stores `last_prog: Option<Arc<Program>>` — the last-seen `Arc`
(not a raw pointer). On each `process()` call it compares with `Arc::ptr_eq`:

```rust
let changed = match &self.last_prog {
    None => true,
    Some(prev) => !Arc::ptr_eq(prev, prog),
};
```

This avoids all `unsafe` and `#[allow]` attributes. `Arc::ptr_eq` is a single
pointer comparison — essentially free. On change:

1. `regs.clear(); regs.resize(prog.num_regs, 0.0)` — reallocate the register file
   (the ONLY allowed allocation site).
2. Set `regs[srate_reg] = sample_rate` so `@init` can use `srate`.
3. Call `run_init(prog, &mut self.regs, BUDGET)` — one-shot init.
4. `self.last_prog = Some(prog.clone())` — clone the Arc (refcount bump, not heap).

## RT-safety of the steady path

Once a program is stable (no pointer change), `process()` does only:

- `self.slot.load()` — one atomic pointer load (ArcSwap's `load_full`-free fast path).
- `Arc::ptr_eq` — one pointer compare.
- Per-frame: `run_sample(prog, &mut self.regs, &mut spl, BUDGET)` — the VM's fixed
  stack array on the Rust call stack + the caller's `&mut [f32]` register slice.
  No allocation, no locks, no I/O.
- `flush(x).clamp(-4.0, 4.0)` — two arithmetic ops.

The VM's per-sample instruction budget (`BUDGET = 100_000`) bounds any runaway
user loop, making arbitrary user-written scripts audio-thread-safe.

## Signature change to `standard_with_ir` (Task 7 note)

`standard_with_ir` now takes a fifth parameter:

```rust
pub fn standard_with_ir(
    sample_rate: f32,
    channels: usize,
    ir_slot: IrSlot,
    gr_meter: Arc<CompanderMeter>,
    script_slot: ScriptSlot,   // NEW — added after existing params
) -> Self
```

`standard()` is unchanged (zero-arg convenience); it calls `standard_with_ir` with
`empty_script_slot()`.

**Task 7 must update `hm-audio/src/engine.rs`** — the single call site at line 232
(`ProcessChain::standard_with_ir(sample_rate, channels, ir_slot, gr_meter)`) now
requires a fifth argument. `cargo build -p hm-audio` currently fails with E0061
(4 args, 5 expected). This is expected and documented here per the brief.

## Tests added and results

File: `crates/hm-dsp/src/script_stage.rs`

| Test | Purpose | Result |
|---|---|---|
| `disabled_is_identity` | `enabled=false` + no program → buffer bit-exact unchanged | PASS |
| `no_program_is_identity` | Enabled but slot holds `None` → identity | PASS |
| `gain_program_halves` | `spl0*0.5; spl1*0.5;` compiled + stored → all samples halved | PASS |
| `stays_bounded` | Multiply by 100000 → output clamped to ±4.0 | PASS |
| `program_change_detected` | Swap ×2 → ×0.25 program; second takes effect | PASS |

Chain tests added to `lib.rs`:

| Test | Result |
|---|---|
| `standard_chain_includes_script` | `chain.len() >= 12` | PASS |

`cargo test -p hm-dsp`: **145 passed, 0 failed**
`cargo clippy -p hm-dsp --all-targets -- -D warnings`: **clean**

## Concerns / follow-up notes

- **hm-audio breaks** as expected (see above). Task 7 must pass `engine.script_slot()`
  as the fifth arg to `standard_with_ir`.
- The `ScriptProcessor::new` constructor ignores its `channels` arg (same pattern as
  `Convolver::with_slot` which also ignores `_channels`). The register file is sized
  per-program, not per-channel — this is correct.
- The `flush()` threshold (`1e-18`) matches `room.rs`; it prevents denormal slowdowns
  in the VM's register file feedback paths.
