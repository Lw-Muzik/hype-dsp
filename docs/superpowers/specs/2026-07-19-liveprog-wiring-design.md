# LiveProg wiring — design

**Date:** 2026-07-19
**Status:** approved, not yet implemented

## The problem

`hm-dsp` ships a complete EEL2-subset VM — `script/{lexer,parser,compiler,vm}.rs`
— and `hm-core::EngineState` carries a `ScriptState { enabled, source }`. Neither
is reachable. `hm-dsp/src/lib.rs` declares `pub mod script;` and **nothing
consumes it**: no chain stage, no Tauri command, no IPC, no UI. A user cannot run
a script, and the persisted `script.source` is never read back by anything.

The missing half exists on `origin/feat/liveprog-eel`, which was never merged and
is now ~51k lines behind `main`. This ports the wiring from that branch onto
main's VM rather than merging the branch, whose own VM is an earlier, smaller
version of the one main already has.

## Why this is a port and not a rewrite

The branch's stage was written against the same API main exposes. Verified
before designing:

| The stage needs | main provides |
|---|---|
| `script::{run_init, run_sample, Program}` | yes, same names |
| `Program.num_regs`, `Program.srate_reg` | yes, same fields |
| `AudioProcessor::{prepare, process, set_params}` | unchanged |
| a slot after `Saturation`, before `Gain` | still unoccupied |

So `script_stage.rs` moves essentially verbatim. The design work is in the two
places main has moved on: how the chain is constructed, and what happens when
state is restored rather than typed.

## Architecture

### `hm-dsp` — the stage

`script_stage.rs`, ported:

- **`ScriptSlot`** = `Arc<ArcSwap<Option<Arc<Program>>>>`. The command thread
  stores; the audio thread loads once per block. Mirrors the convolver's
  `IrSlot`, which is the existing precedent for publishing to the audio thread.
- **Program-change detection** by `Arc::ptr_eq` against the last-seen `Arc`. On
  change — and only then — the register file is resized and `run_init` runs.
  This is the sole point where the stage may allocate.
- **Steady path** is `load()` + `run_sample()`: no locks, no allocation, no I/O.
- **`BLOCK_BUDGET`** (2,000,000 ops) is shared across every frame in one
  `process()` call. Once spent, remaining frames pass through unchanged, so a
  runaway `while(1)` costs a bounded few µs per callback instead of an xrun.
- **Output** is flush-denormaled and clamped to ±4.0; the master limiter
  downstream owns the real ceiling.

Position: after `Saturation`, before `Gain` → `Limiter`.

### `hm-dsp` — `ChainSlots`

`standard_with_ir` currently takes `(sample_rate, channels, ir_slot, gr_meter)`
and would take a fifth. Fold the externally-owned handles into one struct:

```rust
pub struct ChainSlots {
    pub ir: IrSlot,
    pub compander_meter: Arc<CompanderMeter>,
    pub script: ScriptSlot,
}
```

Three positional slots is where a call site stops being readable, and every
existing caller already passes them together. Scoped to this: no other chain
refactoring.

### `hm-audio` — engine

- The engine owns the `ScriptSlot` and hands it to the chain via `ChainSlots`.
- `compile_script(source) -> Result<(), ScriptError>` compiles on whichever
  thread calls it and publishes the resulting `Arc<Program>`. The only guarantee
  that matters: it is never the audio thread. The audio thread's sole
  interaction with a program is an atomic load.
- `set_script(enabled)` toggles the stage without recompiling.

### `hm-audio` — recompiling on restored state

**The gap the branch never had to solve.** `ScriptState` is part of the
serializable `EngineState`, so `source` and `enabled` already survive a restart
and are already captured by whole-chain presets. But only the source is
serialized — the compiled `Program` cannot be — and nothing recompiles it. Left
alone, the card would show a script enabled, with its text, while the chain runs
identity: the UI states something false.

`set_state` already has the precedent. It re-syncs the live crossfade atomic
there for the same reason — a value that lives outside the serializable state and
must be rebuilt when state is replaced:

```rust
pub fn set_state(&self, new_state: EngineState) {
    self.crossfade.store(...);          // existing
    // + recompile new_state.script.source into the slot
}
```

This covers both paths that replace state wholesale: launch restore, and
`chain_preset_apply`. A preset whose script no longer compiles publishes nothing
and leaves the previous program in place rather than failing the apply — a
preset is a sound, and one bad field should not reject the other twenty.

Recompilation is driven by `source` being non-empty, **not** by `enabled`. A
disabled script still compiles on restore, so switching the toggle on is
immediate and does not silently require a trip through Apply first. The stage
reads `enabled` separately and stays identity until it is set.

### Tauri + frontend

- `engine_script_compile(source)` and `engine_set_script(enabled)`, both
  **async**. Compilation is cheap, but this codebase has twice shipped a sync
  command that froze the UI (lyrics, mixer icon extraction); async costs nothing
  and forecloses it.
- `ipc.ts`: `engineScriptCompile`, `engineSetScript`.
- `stores/engine.ts`: `setScriptEnabled`, plus a compile action that surfaces the
  `ScriptError` rather than swallowing it.
- `ScriptCard.tsx` ported as-is: toggle, monospace `<textarea>`, error box,
  Apply, example chips — on main's tokens. Rendered from `EnhancerView`.

Compile errors carry `line`/`col`. The plain textarea shows them as text
(`[3:8] unknown variable 'gg'`) and does not mark the line; a line-number rail
and inline marker were considered and deliberately deferred until the feature has
been used.

## Error handling

| Failure | Behaviour |
|---|---|
| Source does not compile (explicit Apply) | `IpcError("script", "line:col: msg")`; slot unchanged, toggle not flipped on |
| Source does not compile (restore / preset) | Slot unchanged, no error surfaced — the app is starting or applying a sound, not authoring |
| No program loaded | Identity pass-through |
| Stage disabled | Identity pass-through, VM never entered |
| Script exceeds the block budget | Remaining frames in that callback pass through unchanged |
| Script emits NaN / huge values | Flush-denormaled and clamped to ±4.0 before the limiter |

## Testing

Stage (`hm-dsp`):
- identity when disabled, and when no program is loaded
- `@init` runs exactly once per program change, not per block
- budget exhaustion mid-block leaves the remaining frames untouched
- output clamped to ±4.0
- steady path does not allocate

Engine (`hm-audio`):
- `set_state` with a compiling source publishes a program — the regression guard
  for the restore/preset gap
- `set_state` with a non-compiling source leaves the previous program in place

Frontend:
- a compile error surfaces and does not enable the stage

## Explicitly not in scope

- The VM itself. It is already on main and already tested; this touches none of
  it.
- Syntax highlighting, line-number rail, inline error markers.
- Any other chain refactoring beyond `ChainSlots`.
- Merging `origin/feat/liveprog-eel`. After this lands, that branch holds nothing
  main lacks and can be deleted.

## Known limits

None of this is ear-testable from here. A script that compiles and stays within
budget is not a script that sounds correct, and the RT-safety claims are
structural (no allocation on the steady path) rather than measured under load.
