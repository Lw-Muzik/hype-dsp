# LiveProg — EEL2-subset scripting DSP stage — Design

**Date:** 2026-06-24
**Status:** Approved (approach B chosen by user)
**Scope:** Add a user-programmable DSP stage to HypeMuzik Desktop: the user writes a small EEL2-style script that compiles to opcodes and runs per-sample. Closes the JamesDSP LiveProg/EEL gap (#5). **Approach B:** a custom EEL2-subset interpreter written in pure Rust (no C/FFI), safe and fully unit-testable; full `EEL_VM` compatibility (FFT/convolution/advanced routines) is an explicit non-goal / future v2.

## 1. Goal
Let power users write custom effects in EEL2-like syntax (gain, tremolo, soft-clip, simple filters, ring-mod, etc.) without recompiling. The script source is small text — serializable into `EngineState` and capturable by chain presets.

## 2. Language subset (EEL2-inspired)
- **Sections:** `@init` (runs once when the script (re)compiles / on prepare) and `@sample` (runs once per sample, per channel-pair). Optional `@block` (once per processing block) — v1 may skip `@block`.
- **Samples:** `spl0` (left), `spl1` (right) — readable + assignable (the script reads the input sample and writes the output). Mono mirrors `spl0`.
- **Builtins/vars:** `srate` (sample rate, read-only), `$pi`, `$e`. Named user variables (lowercase identifiers) persist across `@sample` calls (EEL2 semantics — they are global registers, initialized in `@init`).
- **Operators:** `+ - * / % ^` (^ = pow), unary `-`, comparison `== != < <= > >=`, logical `&& || !`, assignment `=` (and `+= -= *= /=` if cheap), grouping `( )`, statement separator `;`.
- **Functions:** `sin cos tan asin acos atan atan2 sqrt exp log log10 pow abs min max floor ceil round sign fmod` — a fixed builtin table.
- **Control flow:** `if(cond, then_expr, else_expr)` (EEL2 ternary-ish) AND a bounded `while(cond) ( body )` / `loop(n, body)` — with a hard per-sample instruction budget that aborts runaway loops.
- **NOT in v1:** memory/arrays (`x[i]`), FFT/convolution/fft_*, fractional delay, string ops, user-defined functions, `@gfx`, sliders/MIDI. (These are the `EEL_VM`-only features; v2 via FFI if wanted.)

## 3. Architecture
`crates/hm-dsp/src/script/` (a sub-module):
- `lexer.rs` — source `&str` → `Vec<Token>` (numbers, idents, operators, section markers `@init`/`@sample`, punctuation). Pure, testable; reports position on bad tokens.
- `parser.rs` — tokens → AST (`Section { init: Vec<Stmt>, sample: Vec<Stmt> }`, `Stmt = Assign | Expr`, `Expr` tree with binops/calls/vars/consts). Reports parse errors with position + message.
- `compiler.rs` — AST → a flat **`Vec<Op>` opcode program** + a symbol table mapping variable names → register indices (a fixed `Vec<f32>` register file sized at compile time). Builtins resolve to `Op::Call(BuiltinId)`. `if`/`while`/`loop` compile to jumps with a recorded max-iteration guard.
- `vm.rs` — the RT evaluator: `Program { init_ops, sample_ops, num_registers, ... }`; `run_init(&mut regs)`; `run_sample(&mut regs, spl: &mut [f32; 2], budget: &mut u32)`. A small operand stack (fixed-capacity array, NOT a Vec) + the register file. Allocation-free; every `while`/`loop` decrements the shared instruction `budget` and aborts (returns) at 0. NaN/Inf → 0; output clamp handled by the stage.
- `mod.rs` — `compile(source: &str) -> Result<Program, ScriptError>` (the off-thread entry); `ScriptError { line, col, message }`.
- `crates/hm-dsp/src/script_stage.rs` — `ScriptProcessor` `AudioProcessor`: holds an `Arc<ArcSwap<Option<Arc<CompiledScript>>>>` slot (a `CompiledScript` = `Program` + a per-channel-independent? NO — EEL2 vars are global across L/R within a frame; run the `@sample` block once per frame with spl0/spl1 both available). Per frame: load the program (lock-free), set spl0/spl1 from the buffer, `run_sample` with a fresh per-sample budget, write spl0/spl1 back, clamp(-4,4). Disabled or no-program → bit-exact identity. Denormal-flush the register file between frames is NOT needed (regs are user state) but flush spl on output.

## 4. Real-time safety (critical — arbitrary user code on the audio thread)
- **Off-thread compile:** compilation (lex/parse/compile) happens in the Tauri command, never on the audio thread. The compiled `Program` is published via `ArcSwap` (the convolver-IR pattern). The audio thread only `load()`s and executes opcodes.
- **Allocation-free execution:** the VM uses a fixed operand-stack array + the pre-sized register file (allocated at compile/prepare). No `Vec` growth, no heap, no locks in `run_sample`.
- **Bounded cost / no infinite loops:** a hard per-sample **instruction budget** (e.g. 10_000 ops/sample) is decremented on every executed op; hitting 0 aborts the `@sample` run for that sample (the partial result is clamped). This makes a malicious/buggy `while(1)(...)` safe — it can't hang the audio thread.
- **NaN/Inf guard:** any non-finite intermediate or output → 0/clamped. Output `clamp(-4.0, 4.0)`; the master limiter is downstream.
- **Compile errors never crash:** returned to the UI as `ScriptError`; the stage keeps running the last good program (or identity if none).
- Placement: in `ProcessChain::standard_with_ir` after `Saturation`, before `Gain → Limiter` (script is a color/processing stage; limiter is the final safety net) → chain becomes 12 stages.

## 5. State / wiring
- `ScriptState { enabled: bool, source: String }` in `hm-core` (serde camelCase, `#[serde(default)]`), added to `EngineState` (so it's preset-able + autosaved). Default disabled + empty source.
- The compiled `Program` is NOT in `EngineState` (too big / derived). It lives in an engine-owned `ArcSwap` slot threaded through `standard_with_ir` → `ScriptProcessor::with_slot`, exactly like the convolver `IrSlot`. (`standard` makes a throwaway empty slot.)
- Tauri commands: `engine_script_compile(source: String) -> Result<(), ScriptError>` — compiles off-thread, publishes to the slot, sets `EngineState.script.source`; on error returns `ScriptError` (UI shows it) and does NOT swap the program. `engine_set_script(enabled: bool)` — cheap toggle. (Save updates source; toggling enabled is separate, mirroring other stages.)
- `engine.state()`/`set_state` already carry `script` (via EngineState); applying a chain preset with a script re-compiles it (apply path triggers a compile of the preset's source).

## 6. UI
`src/features/enhancer/ScriptCard.tsx` — a code editor: a monospace `<textarea>` for the source, a **Compile** button → `engine_script_compile` (on success toast "Compiled", on error show the `ScriptError` line/col/message inline), an **Enable** Switch, and 3 built-in **example scripts** (a gain trim, a tremolo, a soft-clip) the user can load into the editor. No syntax highlighting in v1 (textarea). Surfaced errors via the error panel + toast.

## 7. Testing
- **lexer:** tokenizes numbers/idents/operators/sections; errors on bad chars with position.
- **parser:** parses `@init`/`@sample`, expressions with precedence (`1+2*3` → 7), `if(...)`, assignment; reports errors.
- **compiler:** a known script compiles to the expected opcode count / a round-trip eval; unknown function/var → error.
- **vm (the core):** 
  - `gain script` (`@sample spl0 = spl0*0.5;`) halves the signal.
  - `@init` runs once (a counter in @init stays 1 across many @sample calls).
  - math builtins produce correct values (sin/sqrt/etc. within epsilon).
  - **instruction budget** aborts `while(1)(x=x+1)` without hanging (the test asserts it returns within the budget, output finite).
  - NaN guard: `spl0 = 0/0` → 0 (finite output).
  - precedence/associativity correct.
- **ScriptProcessor stage:** disabled/no-program = bit-exact identity; a loaded gain script halves; output bounded; `process` allocation-free.
- Gates: `cargo test -p hm-dsp`/`-p hm-core`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

## 8. Build order (tasks)
1. `ScriptState` (hm-core) + `EngineState` + TS mirror.
2. Lexer (`script/lexer.rs`) + tests.
3. Parser (`script/parser.rs`, AST) + tests.
4. Compiler (`script/compiler.rs`, AST→opcodes + symbol table) + `Op`/`Program`/`ScriptError` types + tests.
5. VM (`script/vm.rs`) RT evaluator (budget, NaN guard, allocation-free) + tests.
6. `ScriptProcessor` stage (`script_stage.rs`) + ArcSwap program slot + chain insert + tests.
7. Engine wiring (program slot threaded; off-thread compile) + Tauri commands (`engine_script_compile`, `engine_set_script`) + registration.
8. TS IPC + store + `ScriptCard` (editor + compile + errors + examples) + render.

## 9. Risks & mitigations
- **RT-safety of arbitrary code** → instruction budget + allocation-free opcodes + bounded loops + NaN guard + clamp + downstream limiter. The single most important property; tested by the runaway-loop + NaN tests.
- **Parser/compiler complexity** → keep the grammar small (precedence-climbing expression parser); thorough unit tests at each layer (lexer/parser/compiler/vm separately).
- **Performance of interpreted opcodes** → fine for moderate scripts at 48 kHz; the budget caps worst case; disabled = zero cost. Heavy scripts are the user's responsibility (the budget keeps the app safe).
- **Compatibility expectations** → document that this is an EEL2 SUBSET; advanced `EEL_VM` scripts (FFT/convolution/arrays) won't run. v2 = `EEL_VM` C-FFI for full compat.

## 10. Non-goals (v1)
Arrays/memory, FFT/convolution/advanced DSP builtins, user functions, `@gfx`/graphics, sliders/MIDI, syntax highlighting, JIT. Future v2 could FFI `james34602/EEL_VM` for full JamesDSP-LiveProg compatibility.
