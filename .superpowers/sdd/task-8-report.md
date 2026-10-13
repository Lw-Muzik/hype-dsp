# Task 8 Report: TS IPC, Store Action, ScriptCard Editor UI

## Files modified / created

| File | Action |
|------|--------|
| `src/lib/types.ts` | Added `ScriptState` interface; added `script: ScriptState` field to `EngineState` |
| `src/lib/ipc.ts` | Added `engineScriptCompile(source)` and `engineSetScript(enabled)` |
| `src/stores/engine.ts` | Added `engineSetScript` import; added `setScriptEnabled` to interface + implementation; added `script` to `defaultEngineState` |
| `src/features/enhancer/ScriptCard.tsx` | **Created** — the full LiveProg editor card |
| `src/features/enhancer/EnhancerView.tsx` | Imported and rendered `<ScriptCard />` after `<SaturationCard />` |

## Builtin verification & example scripts

Checked `crates/hm-dsp/src/script/compiler.rs` — `Builtin::Tanh` exists at line 85, confirmed via `from_name("tanh")` at line 113.

Three example scripts (load-only, no auto-compile):
- **Gain**: `spl0=spl0*0.7; spl1=spl1*0.7;` — uses no builtins
- **Tremolo**: uses `sin` and `$pi` constant — both confirmed present
- **Soft clip**: `@sample spl0=tanh(spl0*2)/tanh(2); spl1=tanh(spl1*2)/tanh(2);` — uses `tanh` (confirmed)

All examples only use builtins present in the `Builtin` enum.

## Error panel flow

1. User clicks **Compile** → `engineScriptCompile(editorText)` is called
2. On **success**: `toast.success("Compiled")`, store source updated via `useEngineStore.setState`, error cleared
3. On **failure**: rejection message from `ipcErrorMessage(e)` displayed in a red inline panel (same `CircleAlert` + `border-danger/30 bg-danger/10` styling as the audio-source error panel)
4. Error clears automatically when the user types (via `onChange`) or clicks an example button

## TypeScript result

`pnpm tsc --noEmit` — **0 errors**

The worktree did not have `ScriptState` in types.ts or `script` in EngineState (branch predates those additions from Tasks 1–7 being merged). Added both cleanly.

## Concerns / notes

- `ScriptCard` seeds `editorText` from `state.script.source` at mount only (local useState). This is intentional — the textarea is a local editor; compilation pushes to the store.
- `useEngineStore.setState` is called directly (not through an action) to update `script.source` on successful compile, keeping the pattern lightweight without a separate `setScriptSource` action the brief didn't ask for.
- The `Button` and `Switch` components are used from `@/components/` — same as all other enhancer cards. No new dependencies.
