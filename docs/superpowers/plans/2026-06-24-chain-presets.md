# Whole-Chain DSP Preset Manager — Implementation Plan

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development.

**Goal:** Named, exportable/importable presets of the ENTIRE enhancement chain (`EngineState`) — save the current sound, apply a saved one, delete, and export/import for sharing. Distinct from the EQ-only `PresetStore`. Closes JamesDSP "preset manager" gap (#7).

**Architecture:** A JSON-file-backed `ChainPresetStore` (in hm-core, mirrors how the SQLite `PresetStore` is `app.manage`d) holding `Vec<ChainPreset { id, name, state: EngineState }>`. Tauri commands list/save/apply/delete/export/import. Apply preserves the user's current `power` + `master_volume` (a preset is a *sound*, not your volume). First task hardens `EngineState` deserialization so presets (and the existing `engine-state.json` autosave) survive future field additions.

## Global Constraints
- `ChainPreset`/`EngineState` serde camelCase; `ChainPreset` mirrored in `src/lib/types.ts`.
- Forward-compat: `EngineState` (and its sub-state structs) deserialize missing fields via `Default` (so old presets/autosave don't break when a stage is added).
- Apply preserves current `power` + `master_volume`; everything else comes from the preset.
- Store is file-backed JSON in `app_data_dir/chain-presets.json`; `app.manage`d like `PresetStore`.
- Gates per crate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

---

## Task 1: `EngineState` forward-compat (`#[serde(default)]`)
**Files:** `crates/hm-core/src/types.rs`; test there.
**Why:** today `serde_json::from_str::<EngineState>` FAILS on any missing field; the autosave silently discards on failure (settings reset) and presets would become unloadable when a stage is added.

- [ ] **Failing test** — append to types.rs tests:
```rust
    #[test]
    fn engine_state_deserializes_with_missing_fields() {
        // A partial blob (as if saved before newer stages existed) must load,
        // filling the absent fields from Default — not error.
        let json = r#"{"power":true,"masterVolume":1.0,"eq":{"enabled":true,"preGain":0.0,"bands":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}"#;
        let st: EngineState = serde_json::from_str(json).expect("partial EngineState must deserialize");
        assert!(st.power);
        assert!(!st.saturation.enabled);   // absent → Default (disabled)
        assert!(!st.compander.enabled);     // absent → Default
        assert!(!st.convolver.enabled);     // absent → Default
    }
```
(Add `use serde_json;` in the test if needed — hm-core already depends on serde_json.)
- [ ] Run → fails (missing-field deserialize error).
- [ ] **Implement**: add `#[serde(default)]` to the `EngineState` struct container (after the existing `#[serde(rename_all = "camelCase")]`). ALSO add `#[serde(default)]` to EACH sub-state struct that can grow (`EqState`, `BassBoostState`, `SpatializerState`, `Surround3DState`, `RoomState`, `ConvolverState`, `CompanderState`, `SaturationState`, `HeadphoneCorrectionState`, `OutputState`, `PlaybackState`) so a present-but-partial nested object also fills missing fields from Default. (Each already derives `Default`.) The bands array stays required within EqState (fixed-size); that's fine — the test provides it.
- [ ] Run → passes. `cargo test -p hm-core` whole crate green. `cargo clippy -p hm-core --all-targets -- -D warnings` clean. Commit `fix(core): EngineState forward-compat — default missing fields on deserialize`.

---

## Task 2: `ChainPreset` + `ChainPresetStore` (hm-core, JSON file)
**Files:** Create `crates/hm-core/src/chain_presets.rs`; modify `crates/hm-core/src/lib.rs` (`pub mod chain_presets;` + re-export); `src/lib/types.ts`; test in chain_presets.rs.
**Produces:**
- `pub struct ChainPreset { pub id: String, pub name: String, pub state: EngineState }` (serde camelCase, Clone).
- `pub struct ChainPresetStore { path: PathBuf }` with: `open(path: &Path) -> Self`; `list(&self) -> Result<Vec<ChainPreset>, HmError>` (reads the JSON file; empty/absent → `[]`); `save(&self, name: &str, state: EngineState) -> Result<ChainPreset, HmError>` (assigns an id — a millis timestamp or a counter; appends; writes); `delete(&self, id: &str) -> Result<(), HmError>` (filters + writes); `upsert_imported(&self, preset: ChainPreset) -> Result<ChainPreset, HmError>` (add an imported preset, assigning a fresh id to avoid collisions).
- File format: a JSON array of `ChainPreset` at `path` (write-then-rename for safety, mirroring engine-state autosave). No SQLite — keep it simple; the whole list is small.
- `id` generation: avoid `Date::now()` issues — use `std::time::SystemTime::now()` millis (hm-core already uses `SystemTime`/`UNIX_EPOCH` in store.rs) or a max-existing-id+1 counter. (NOTE: the workflow `Date.now` ban is for WORKFLOW scripts only, not app code — `SystemTime` is fine here.)

- [ ] **Tests** (temp-file-backed):
  - `save_then_list_roundtrips`: open at a temp path, save("Warm", state_A), save("Punch", state_B); list() returns 2 with matching names + states.
  - `delete_removes`: save 2, delete one by id, list() returns the other.
  - `list_empty_when_absent`: open at a nonexistent path → list() == `[]` (no error).
  - `upsert_imported_assigns_fresh_id`: import a ChainPreset whose id collides with an existing one → stored with a new unique id; list() has both.
  - `persists_across_reopen`: save, drop, re-open the same path, list() still has it.
- [ ] **TS mirror** in types.ts: `export interface ChainPreset { id: string; name: string; state: EngineState; }`.
- [ ] TDD → implement → pass → clippy clean → commit `feat(core): ChainPreset + JSON-file ChainPresetStore`.

---

## Task 3: Tauri commands + store wiring
**Files:** `src-tauri/src/lib.rs` (manage the store + register), `src-tauri/src/commands/engine.rs` (or a new `commands/presets.rs`).
**Produces:** commands `chain_preset_list`, `chain_preset_save`, `chain_preset_apply`, `chain_preset_delete`, `chain_preset_export`, `chain_preset_import`.

- [ ] **Wire the store**: in lib.rs setup (near `PresetStore::open(...)` + `app.manage`), open `ChainPresetStore::open(&dir.join("chain-presets.json"))` and `app.manage(chain_store)`.
- [ ] **Commands** (take `State<'_, ChainPresetStore>` + `State<'_, AudioEngine>` as needed; map `HmError`→`IpcError` like existing preset commands):
  - `chain_preset_list() -> Result<Vec<ChainPreset>, IpcError>` → `store.list()`.
  - `chain_preset_save(engine, store, name: String) -> Result<ChainPreset, IpcError>` → `store.save(&name, engine.state())` (capture the live `EngineState` via the engine's `state()`/`engine_get_state` equivalent).
  - `chain_preset_apply(engine, store, id: String) -> Result<(), IpcError>` → find the preset in `store.list()`; build the applied state = preset.state but with `power` and `master_volume` taken from the CURRENT `engine.state()`; `engine.set_state(applied)`. (Preset is a sound, not your volume.)
  - `chain_preset_delete(store, id: String) -> Result<(), IpcError>` → `store.delete(&id)`.
  - `chain_preset_export(store, id: String, path: String) -> Result<(), IpcError>` → find preset; `serde_json::to_string_pretty(&preset)` → write to `path`.
  - `chain_preset_import(store, path: String) -> Result<ChainPreset, IpcError>` → read `path`; `serde_json::from_str::<ChainPreset>` (forward-compat via Task 1); `store.upsert_imported(preset)`.
  - Register ALL six in the `tauri::generate_handler![...]` list.
- [ ] Confirm `AudioEngine` exposes a way to read the current full state (`engine.state()` — it exists per engine.rs; if the command needs it, use it). `engine.set_state(EngineState)` exists for apply.
- [ ] `cargo build -p hypemuzik` compiles; `cargo clippy -p hypemuzik --all-targets -- -D warnings` clean (your additions). Commit `feat(tauri): whole-chain preset commands (list/save/apply/delete/export/import)`.

---

## Task 4: TS IPC + store + Presets UI card
**Files:** `src/lib/ipc.ts`, `src/stores/engine.ts`, create `src/features/enhancer/PresetsCard.tsx`, render in `EnhancerView.tsx`. Uses `@tauri-apps/plugin-dialog` (`open`/`save`) for export/import file paths (already a dep).
- [ ] **ipc.ts**: wrappers `chainPresetList(): Promise<ChainPreset[]>`, `chainPresetSave(name): Promise<ChainPreset>`, `chainPresetApply(id): Promise<void>`, `chainPresetDelete(id): Promise<void>`, `chainPresetExport(id, path): Promise<void>`, `chainPresetImport(path): Promise<ChainPreset>`. (+ `ChainPreset` type import.)
- [ ] **store** (engine.ts): after `chain_preset_apply`, refresh the engine state from the backend (call `engineGetState()` and `set({ state })`, OR have apply return the applied state) so the UI reflects the applied preset. Keep it simple: an `applyChainPreset(id)` action that calls the IPC then re-hydrates via the existing `engine_get_state` path.
- [ ] **PresetsCard.tsx**: a `<Card title="Presets" icon={...}>` with: a list of saved presets (name + Apply + Delete + Export buttons each); a "Save current as…" row (text input + Save button using the typed name); an "Import…" button (file `open` dialog → `chainPresetImport`). Export uses the `save` dialog to pick a path. Refresh the list after save/delete/import. No native `<select>`; match card styling; every control accessible. Surface errors via the app's `toast`.
- [ ] Render `<PresetsCard />` in EnhancerView (e.g. near the top or bottom of the enhancer cards).
- [ ] `pnpm tsc --noEmit` clean. Commit `feat(ui): whole-chain preset manager card`.

---

## Final
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (4 crates), `pnpm tsc --noEmit` green. Whole-branch review. Open a PR (matches the repo's PR workflow) for the user to merge.

## Notes
- Disk: `target/` grew huge this session from cross-compiles — if a build fails on space, `rm -rf target/<triple>` (android/windows) + `target/release` (regenerate on demand).
- Apply-preserves-volume/power is deliberate. The convolver IR is referenced by path inside `EngineState.convolver.ir_id`; an imported preset from another machine may point at a missing IR file — applying still works (convolver just has no IR loaded until the path resolves); acceptable, note it in the card if cheap.
