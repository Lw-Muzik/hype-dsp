# Task 3 Report: Tauri Commands + Store Wiring

## Commit

`c05aa5e` — feat: wire ChainPresetStore + add 6 chain-preset Tauri commands (task 3)

## Files Changed

- `src-tauri/src/commands/chain_presets.rs` — new file, 6 Tauri command implementations
- `src-tauri/src/commands/mod.rs` — added `pub mod chain_presets;`
- `src-tauri/src/lib.rs` — added `ChainPresetStore` import + Mutex wiring + 6 handler registrations

## Mutex Wiring (lib.rs)

`ChainPresetStore` has no internal lock, so it is wrapped in `std::sync::Mutex<ChainPresetStore>` when managed:

```rust
if let Ok(dir) = app.path().app_data_dir() {
    let _ = std::fs::create_dir_all(&dir);
    let chain_store = ChainPresetStore::open(&dir.join("chain-presets.json"));
    app.manage(Mutex::new(chain_store));
} else {
    // Fallback: temp dir so the app still runs even if app_data_dir fails
    let chain_store = ChainPresetStore::open(&std::env::temp_dir().join("hm_chain_presets.json"));
    app.manage(Mutex::new(chain_store));
}
```

The block is placed immediately after the EQ `PresetStore` block, mirroring that pattern. Each command acquires the lock via `store.lock().map_err(|_| IpcError::new("lock", "preset store poisoned"))?`.

## The 6 Commands

### `chain_preset_list`
Takes `State<'_, Mutex<ChainPresetStore>>`. Locks, calls `store.list()`, maps `HmError → IpcError`. Returns `Vec<ChainPreset>`.

### `chain_preset_save`
Takes engine + store state + `name: String`. Reads current state with `engine.state()` BEFORE locking the store (minimizes lock hold time). Calls `store.save(&name, current)`. Returns the new `ChainPreset` with its generated id.

### `chain_preset_apply`
Takes engine + store + `id: String`. Apply-preserve-volume/power logic:
1. Reads `current = engine.state()` before locking store
2. Locks store, calls `store.list()`, finds preset by id (returns `not_found` error if absent)
3. Takes `applied = preset.state`, then overrides `applied.power = current.power` and `applied.master_volume = current.master_volume`
4. `drop(store)` — releases lock before calling `engine.set_state(applied)`

This preserves the user's bypass toggle and output level across preset recall (a preset is a sound, not a volume setting).

### `chain_preset_delete`
Takes store + `id: String`. Locks, calls `store.delete(&id)`. Returns `not_found` error if preset doesn't exist (propagated from `HmError::NotFound`).

### `chain_preset_export`
Takes store + `id: String` + `path: String`. Locks, calls `store.list()`, finds preset by id, serializes with `serde_json::to_string_pretty`, writes to `path` via `std::fs::write`. File/serde errors are wrapped in `IpcError::new("io"/"serde", ...)`.

### `chain_preset_import`
Takes store + `path: String`. Reads file with `std::fs::read_to_string`, parses as `ChainPreset` with `serde_json::from_str` (forward-compat from Task 1 `#[serde(default)]`), calls `store.upsert_imported(preset)` which assigns a fresh id to avoid collisions.

## HmError → IpcError Handling

`From<HmError> for IpcError` is already implemented in `hm-core/src/error.rs`. Terminal returns use `.map_err(Into::into)` which the compiler resolves unambiguously from the function return type. Intermediate `?` expressions (where multiple `From` impls exist — `AudioError`, `HmError`, `PlatformError` — causing E0282/E0283 ambiguity) use the explicit form `.map_err(|e: hm_core::HmError| IpcError::from(e))` to nail the type.

## Build Output

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 17.58s
```

No errors, no warnings in new code.

## Clippy Output

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 29s
```

Exit code 0. Zero `warning[...]` lines — clean for all additions and the existing codebase. One informational note about `block v0.1.6` (a transitive dep, pre-existing, not actionable).

## Test Output (`cargo test -p hm-audio`)

```
test result: ok. 31 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

## Concerns

1. **Worktree started at wrong commit**: The worktree was initialized at `a6bc6a4` (before Tasks 1 and 2), not at `557be44` (the Task 3 starting point). This was discovered when the build failed with "no `ChainPresetStore` in the root". Fixed by merging `feat/chain-presets` into the worktree branch first, then restoring the task-3 changes via `git stash`/`pop`. The final commit is on top of all previous branch work.

2. **Sidecar binary not in worktree**: The `binaries/hm-visualizer-aarch64-apple-darwin` sidecar (required by Tauri's build script) doesn't live in git — it's a build artifact. Copied from the main checkout for the build. This is a pre-existing project infrastructure issue, not introduced by this task.

3. **Convolver IR path on imported presets**: As noted in the plan, an imported preset may reference a convolver IR path (`state.convolver.ir_id`) that doesn't exist on the importing machine. Applying the preset still works (DSP chain stays valid); the convolver stage just has no IR loaded until the user re-loads one. No special handling added — acceptable behavior, as noted in the plan.

4. **No disk fallback for `chain_preset_apply` lock drop timing**: The lock is dropped before `engine.set_state()` to avoid holding it during a potential DSP state swap. This is the correct approach — the preset is already extracted at that point.
