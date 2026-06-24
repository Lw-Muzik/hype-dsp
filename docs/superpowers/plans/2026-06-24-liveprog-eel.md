# LiveProg (EEL2-subset scripting) — Implementation Plan

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development.

**Goal:** A user-programmable DSP stage — EEL2-style script compiled to opcodes, run per-sample, RT-safe.

**Architecture:** `hm-dsp/src/script/` = lexer → parser(AST) → compiler(opcodes) → vm(RT evaluator); a `ScriptProcessor` `AudioProcessor` runs the compiled program from an `ArcSwap` slot (off-thread compile). State source in `EngineState`. Pure Rust, no FFI.

**Tech Stack:** Rust (hm-core, hm-dsp), React+TS+Zustand, Tauri. No new crates.

## Global Constraints
- RT-safety: the VM `run_sample` + `ScriptProcessor::process` NEVER allocate/lock/IO. Operand stack = fixed array; register file pre-sized at compile. Compilation is OFF the audio thread; the `Program` is published via `ArcSwap` (convolver-IR pattern).
- **Instruction budget**: every executed op decrements a per-sample budget (default 10_000); at 0 the `@sample` run aborts → a buggy `while(1)` CANNOT hang the audio thread. Non-finite (NaN/Inf) intermediates/outputs → 0; stage output `clamp(-4,4)`; master limiter downstream.
- Disabled or no program → bit-exact identity.
- `ScriptState` serde camelCase + `#[serde(default)]`, mirrored in TS. Default: `{ enabled: false, source: "" }`.
- Chain placement: after `Saturation`, before `Gain → Limiter` (chain → 12 stages).
- Gates per crate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

---

## Task 1: `ScriptState` (hm-core) + EngineState + TS mirror
**Files:** `crates/hm-core/src/types.rs`, `src/lib/types.ts`; test in types.rs.
**Produces:** `hm_core::ScriptState { enabled: bool, source: String }` default disabled/empty; `EngineState.script`.
- [ ] Failing test:
```rust
    #[test]
    fn script_default_is_disabled_empty() {
        let s = ScriptState::default();
        assert!(!s.enabled);
        assert!(s.source.is_empty());
        assert!(!EngineState::default().script.enabled);
    }
```
- [ ] Implement: add (after another state's Default):
```rust
/// User LiveProg script (EEL2-subset). The compiled program lives engine-side;
/// only the source text is part of the serializable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptState {
    pub enabled: bool,
    pub source: String,
}
```
(NOTE: `#[derive(Default)]` works — `bool`/`String` default to false/"".) Add `pub script: ScriptState,` to `EngineState` (after `saturation`) + `script: ScriptState::default(),` to its Default. TS `src/lib/types.ts`: `export interface ScriptState { enabled: boolean; source: string; }` + `script: ScriptState;` on `EngineState`; fix default-state literals.
- [ ] `cargo test -p hm-core` green; `pnpm tsc --noEmit` clean. Commit `feat(core): add ScriptState to EngineState + TS mirror`.

---

## Task 2: Lexer (`hm-dsp/src/script/lexer.rs`)
**Files:** Create `crates/hm-dsp/src/script/mod.rs` (`pub mod lexer; ...`) + `lexer.rs`; register `pub mod script;` in hm-dsp lib.rs; test in lexer.rs.
**Produces:** `pub enum Token { Num(f64), Ident(String), Op(OpTok), LParen, RParen, Comma, Semicolon, SectionInit, SectionSample, /* SectionBlock */ }` (+ a `Spanned<Token>` carrying `line`/`col`), and `pub fn lex(src: &str) -> Result<Vec<Spanned<Token>>, ScriptError>`. `ScriptError { line: u32, col: u32, message: String }` (define here or in mod.rs; serde-Serialize for the UI).
- Tokenizes: numbers (int/float, `$pi`/`$e` as special constants → `Ident` or a `Const` token), identifiers `[a-z_][a-z0-9_]*` (case-insensitive EEL → lowercase), `spl0`/`spl1`/`srate` are plain idents, operators `+ - * / % ^ = == != < <= > >= && || !` and compound `+= -= *= /=`, `( ) , ;`, section markers `@init`/`@sample` (and `@block` → tokenize but parser may reject in v1), line comments `//`, whitespace skipped. Position tracked for errors.
- [ ] Tests: tokenizes a small script into the expected token list; `@init`/`@sample` recognized; numbers/floats parsed; an illegal char (e.g. `#`) → `ScriptError` with the right line/col; comments + whitespace skipped; multi-char operators (`<=`, `==`, `&&`) lexed as one token.
- [ ] TDD → implement → `cargo test -p hm-dsp script::lexer` pass → clippy clean → commit `feat(dsp): LiveProg lexer`.

---

## Task 3: Parser (`hm-dsp/src/script/parser.rs`, AST)
**Files:** Create `parser.rs` + the AST types; test there.
**Consumes:** `Vec<Spanned<Token>>`. **Produces:** `pub struct Ast { pub init: Vec<Stmt>, pub sample: Vec<Stmt> }`; `pub enum Stmt { Assign { name: String, op: AssignOp, value: Expr }, Expr(Expr) }`; `pub enum Expr { Num(f64), Const(Const), Var(String), Unary(UnOp, Box<Expr>), Binary(BinOp, Box<Expr>, Box<Expr>), Call(Builtin, Vec<Expr>), If(Box<Expr>,Box<Expr>,Box<Expr>), While(Box<Expr>, Vec<Stmt>), Loop(Box<Expr>, Vec<Stmt>) }`; `pub fn parse(tokens: &[Spanned<Token>]) -> Result<Ast, ScriptError>`.
- Precedence-climbing expression parser: `|| < && < comparison < + - < * / % < ^ < unary < primary`. `^` right-assoc. `if(c,a,b)`, `while(cond)(body...)`, `loop(n)(body...)` parse specially. Statements separated by `;`. Sections delimited by `@init`/`@sample`; code before any section → implicitly `@sample`? (decide: require explicit sections; code with no section header is an error OR defaults to @sample — pick: default-to-@sample for a bare script, document it). Unknown function name → still parse as `Call` with the name; resolution/validation happens in the compiler (Task 4) which knows the builtin table.
- [ ] Tests: `1+2*3` parses with `*` bound tighter (eval in Task 5 confirms 7); `2^3^2` right-assoc; `spl0 = spl0 * 0.5;` → an `Assign`; `if(spl0>0, spl0, -spl0)` parses; a bare script (no `@init`/`@sample`) defaults to `@sample`; a missing `)` → `ScriptError` with position; precedence of `&&`/`||`/comparison correct.
- [ ] TDD → implement → `cargo test -p hm-dsp script::parser` pass → clippy clean → commit `feat(dsp): LiveProg parser (AST)`.

---

## Task 4: Compiler (`hm-dsp/src/script/compiler.rs`, AST → opcodes)
**Files:** Create `compiler.rs` + `Op`/`Program` types; test there.
**Consumes:** `Ast`. **Produces:**
```rust
pub enum Op {
    PushConst(f32),
    LoadReg(u16), StoreReg(u16),         // user vars (incl. spl0/spl1/srate mapped to fixed reg ids)
    Add, Sub, Mul, Div, Mod, Pow, Neg,
    Eq, Ne, Lt, Le, Gt, Ge, And, Or, Not,
    Call(Builtin),                        // pops N args, pushes 1
    JumpIfFalse(u32), Jump(u32),          // for if / while / loop (control flow via the operand stack)
    Pop,
}
pub struct Program {
    pub init: Vec<Op>,
    pub sample: Vec<Op>,
    pub num_regs: usize,                  // size of the register file
    pub spl0_reg: u16, pub spl1_reg: u16, pub srate_reg: u16,
}
```
- The compiler walks the AST emitting a postfix opcode stream; a symbol table maps variable names → register indices (spl0/spl1/srate get reserved fixed indices; user vars get the next indices). `if`/`while`/`loop` emit `JumpIfFalse`/`Jump` with patched targets (record positions, backpatch). Validate builtins against a fixed `Builtin` table; unknown function or assigning to `srate` (read-only) → `ScriptError`. `loop(n)` compiles to a counted loop using a hidden register; `while(cond)` to a back-jump (the runtime budget bounds it).
- `pub fn compile(src: &str) -> Result<Program, ScriptError>` in `script/mod.rs` chains lex→parse→compile.
- [ ] Tests: `spl0=spl0*0.5` compiles (sample ops non-empty; spl0_reg used); unknown function `foo(1)` → error; assigning `srate=5` → error; an `if` emits a `JumpIfFalse`; num_regs counts the distinct vars; `compile("garbage(")` returns a `ScriptError` (lex/parse error propagates).
- [ ] TDD → implement → `cargo test -p hm-dsp script::compiler` pass → clippy clean → commit `feat(dsp): LiveProg compiler (AST→opcodes)`.

---

## Task 5: VM (`hm-dsp/src/script/vm.rs`) — RT evaluator (the safety-critical core)
**Files:** Create `vm.rs`; test there.
**Consumes:** `Program`, the builtin table. **Produces:** a `Vm`/`Regs` runtime + `pub fn run_init(prog: &Program, regs: &mut [f32])` and `pub fn run_sample(prog: &Program, regs: &mut [f32], spl: &mut [f32; 2], budget: u32) -> ()`.
- Execution: a fixed operand-stack array `[f32; STACK_CAP]` (e.g. 256) — NO Vec; the `regs` slice is the caller's pre-sized register file. `run_sample` writes `spl[0]`/`spl[1]` into the spl registers, runs `sample` ops, reads them back into `spl`. Every executed op decrements a `budget` counter; on 0 → return immediately (partial eval; outputs are whatever's in spl regs, then clamped by the stage). Division by zero / non-finite results → 0 (or push 0). Stack overflow/underflow (shouldn't happen from a valid compile, but guard) → abort the run safely (no panic). Builtins dispatched by `Builtin` id (a match → the f32 math fn).
- `run_init` runs the `init` ops once (e.g. when the program is (re)loaded / on prepare) to initialize user registers; budget-bounded too.
- [ ] Tests (the correctness + SAFETY gate):
  - **gain**: compile+run `@sample spl0=spl0*0.5; spl1=spl1*0.5;` over a signal → output is halved.
  - **init-once semantics**: `@init n=0; @sample n=n+1; spl0=n;` — after K samples spl0==K (init ran once, n persists). (Confirms run_init separate from run_sample + reg persistence.)
  - **precedence**: a script computing `spl0 = 1+2*3` → spl0==7.
  - **builtins**: `spl0 = sqrt(spl0)` / `sin`/`abs`/`min`/`max` produce correct values within epsilon.
  - **RUNAWAY LOOP (critical)**: `@sample while(1) ( n = n+1 );` — `run_sample` RETURNS within the budget (assert it doesn't hang; output finite). This proves the instruction budget makes arbitrary user code RT-safe.
  - **NaN guard**: `spl0 = 0/0` or `spl0 = log(-1)` → spl0 finite (0).
  - **allocation-free**: `run_sample` uses only the fixed stack + caller regs (assert by inspection / no Vec in the hot path).
- [ ] TDD → implement → `cargo test -p hm-dsp script::vm` pass → clippy clean → commit `feat(dsp): LiveProg VM (RT-safe opcode evaluator, instruction budget)`.

---

## Task 6: `ScriptProcessor` stage + ArcSwap program slot + chain insert
**Files:** Create `crates/hm-dsp/src/script_stage.rs`; modify `hm-dsp/src/lib.rs` (export + chain insert); test there.
**Produces:** `pub type ScriptSlot = Arc<ArcSwap<Option<Arc<Program>>>>;` + `pub fn empty_script_slot() -> ScriptSlot`; `pub struct ScriptProcessor` impl `AudioProcessor` with `with_slot(sample_rate, channels, slot)` + `new`; inserted in `standard_with_ir` after `Saturation`, before `Gain`.
- `ScriptProcessor`: holds the slot + a pre-sized register file (`Vec<f32>` sized to the loaded program's `num_regs`, grown only when a new program loads — off the steady path / in set_params when the slot changes, NOT per sample) + the per-sample budget constant. `prepare` sets `srate` reg. `process`: `load()` the program; if `None` or `!enabled` → identity. On a NEW program (pointer changed), run `run_init` once + resize regs. Per frame: `run_sample(prog, &mut regs, &mut [l, r], BUDGET)`; write back; flush/clamp(-4,4). `set_params` reads `params.script.enabled`. (The `srate` register is set in prepare; `num_regs`/init handled on program swap — detect swap by comparing the `Arc` pointer.)
- [ ] Tests: `disabled_is_identity` (no program / disabled = bit-exact); load a gain `Program` into the slot, enable → halves the buffer; output bounded; `process` allocation-free on the steady path (program unchanged).
- [ ] Chain test: `standard_with_ir(...).len() >= 12` (after Saturation insert). Update chain-order doc comments to include `→ Script`.
- [ ] TDD → implement → `cargo test -p hm-dsp` green, `cargo build -p hm-audio` compiles → clippy clean → commit `feat(dsp): ScriptProcessor stage + chain insert`.

---

## Task 7: Engine wiring (off-thread compile + slot) + Tauri commands
**Files:** `crates/hm-audio/src/engine.rs` (thread the `ScriptSlot` like the convolver `IrSlot`; `compile_script`/`set_script`), `src-tauri/src/commands/engine.rs` + `lib.rs`.
**Produces:** `AudioEngine::script_slot()` + `set_script(enabled)` + `compile_script(source) -> Result<(), ScriptError>` (compile OFF the caller/command thread, publish to the slot, update `EngineState.script.source`; on error don't swap). Commands `engine_script_compile(source) -> Result<(), IpcError>` (map ScriptError→IpcError, or return the ScriptError fields) and `engine_set_script(enabled)`.
- Thread the `ScriptSlot` from `AudioEngine::new` → control thread → `Renderer::new` → `standard_with_ir(..., script_slot)` exactly like `ir_slot`/`compander_gr` (add the param; `standard` uses `empty_script_slot()`; update the 2 system-eq `standard` callers indirectly via `standard`, and the 3 `Renderer::new` call sites).
- `compile_script`: `hm_dsp::script::compile(&source)` (off-thread) → on Ok publish `Arc::new(program)` to the slot + `self.update(|s| s.script.source = source; s.script.enabled = true)`; on Err return the ScriptError (UI shows it), slot unchanged.
- [ ] `cargo build -p hypemuzik`, `cargo test -p hm-audio` green, clippy clean. Register both commands. Commit `feat(audio/tauri): LiveProg compile + toggle (off-thread compile, lock-free slot)`.

---

## Task 8: TS IPC + store + `ScriptCard` (editor)
**Files:** `src/lib/ipc.ts`, `src/stores/engine.ts`, create `src/features/enhancer/ScriptCard.tsx`, render in `EnhancerView.tsx`.
- [ ] ipc: `engineScriptCompile(source): Promise<void>` (rejects with the ScriptError on compile failure — surface line/col/message), `engineSetScript(enabled): Promise<void>`. (+ `ScriptState` import if needed.)
- [ ] store: `setScriptEnabled(enabled)` (mirror setRoom toggle); the compile is called directly from the card (it returns errors to display), then update local `state.script`.
- [ ] `ScriptCard.tsx`: a `<Card title="LiveProg (EEL)">` with a monospace `<textarea>` bound to local editor text (seeded from `state.script.source`), an Enable `<Switch>` (→ `engineSetScript`), a **Compile** button (→ `engineScriptCompile(text)`; on success `toast.success("Compiled")` + mark enabled; on error show the error line/col/message in a red inline panel), and a small row of **example** buttons that load preset scripts into the editor: Gain (`spl0=spl0*0.7; spl1=spl1*0.7;`), Tremolo (`@init t=0; @sample t=t+1; g=0.5+0.5*sin(t*6.28*4/srate); spl0=spl0*g; spl1=spl1*g;`), Soft clip (`@sample spl0=tanh(spl0*2)/tanh(2); spl1=tanh(spl1*2)/tanh(2);` — NOTE: if `tanh` isn't in the v1 builtin table, use a script that only uses available builtins, e.g. clip via min/max). No native `<select>`. Match card styling.
- [ ] Render `<ScriptCard />` in EnhancerView. `pnpm tsc --noEmit` clean. Commit `feat(ui): LiveProg script editor card`.

---

## Final
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (4 crates), `pnpm tsc --noEmit` green. Whole-branch review (emphasis on the VM's RT-safety: budget bounds runaway loops, allocation-free, NaN guard). Open a PR.

## Self-review notes
- Spec coverage: state(T1), lexer(T2), parser(T3), compiler(T4), vm(T5), stage+chain(T6), engine+cmd(T7), UI(T8). All map.
- RT-safety is the crux: T5's runaway-loop + NaN + alloc-free tests are the gate; T6's identity + bounded + alloc-free; off-thread compile in T7.
- Builtin table must be consistent across compiler(T4 validation) and vm(T5 dispatch) — define `Builtin` enum once (in T4) and use it in T5. Ensure the example scripts in T8 only use builtins that exist (adjust if `tanh` absent).
- Type consistency: `ScriptState`/`script`, `ScriptError`, `Program`, `Op`, `ScriptSlot`/`empty_script_slot`, `ScriptProcessor`, `engine_script_compile`/`engine_set_script` used identically across tasks.
