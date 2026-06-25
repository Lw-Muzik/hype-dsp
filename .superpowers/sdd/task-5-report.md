## Task 5 Report: `data_saver` flag end-to-end

### Files changed (7)

| File | Change |
|------|--------|
| `crates/hm-core/src/types.rs` | Added `pub data_saver: bool` to `PlaybackState`; added `data_saver: false` to `Default` impl |
| `crates/hm-audio/src/engine.rs` | Added `set_data_saver(&self, on: bool)` after `set_playback`; fixed struct literal in `set_playback` to preserve `data_saver` via `s.playback.data_saver` |
| `src-tauri/src/commands/engine.rs` | Added `engine_set_data_saver` Tauri command mirroring `engine_set_playback` |
| `src-tauri/src/lib.rs` | Registered `commands::engine::engine_set_data_saver` in `generate_handler!` list |
| `src/lib/types.ts` | Added `dataSaver: boolean` to `PlaybackState` interface |
| `src/lib/ipc.ts` | Added `engineSetDataSaver(on: boolean): Promise<void>` |
| `src/stores/engine.ts` | Added `dataSaver: false` to default state literal; spread `s.state.playback` in `setPlayback` to preserve `dataSaver` when updating gapless/crossfade |

### Extra fixes required (not in brief)

`set_playback` in `engine.rs` reconstructed `PlaybackState` with a struct literal — adding `data_saver` to the type caused a compile error there. Fixed by preserving the current value: `data_saver: s.playback.data_saver`.

Similarly, `src/stores/engine.ts` had two inline `PlaybackState` object literals that didn't include `dataSaver`, causing `pnpm tsc --noEmit` errors. Fixed both: default state gets `dataSaver: false`, and `setPlayback` now spreads `s.state.playback` before overriding `gapless`/`crossfadeSecs`.

### Build / check results

```
cargo check -p hypemuzik
  → Finished dev profile (no errors)

cargo test -p hm-core
  → 40 passed; 0 failed

cargo clippy -p hm-audio --all-targets
  → Finished dev profile (no warnings)

pnpm tsc --noEmit
  → (no output = clean)
```

### Commit

`d2831d6  feat: Data Saver (low-bandwidth) flag end-to-end`
Branch: `feat/crossfade-cloud-phone` — pushed to remote.

### Concerns / notes

- `data_saver` is persisted in `engine-state.json` via `EngineState` autosave. Old saved states deserialize cleanly (field is `#[serde(default)]`).
- The flag has no effect yet on streaming behaviour — it is a state/plumbing-only addition per the task spec. Consumers (cloud/phone stream queues) will check `engine.state().playback.data_saver` when that feature is implemented.
