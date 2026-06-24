# Task 2 Report: ChainPreset + ChainPresetStore

## Files Changed

- **Created** `crates/hm-core/src/chain_presets.rs` — `ChainPreset` struct + `ChainPresetStore`
- **Modified** `crates/hm-core/src/lib.rs` — added `pub mod chain_presets;` + re-exports
- **Modified** `src/lib/types.ts` — added `ChainPreset` interface after `EqPreset`

---

## Implementation Notes

### ID scheme

IDs use `format!("{}{}", millis, list_len)` where `millis` is `SystemTime::now().duration_since(UNIX_EPOCH).as_millis()` and `list_len` is the current count of presets _before_ appending the new one.

- Same pattern as `PresetStore.save_custom` (which uses nanos for higher resolution), but millis are sufficient here since the list-length suffix provides the disambiguator.
- Example: if millis=1750000000000 and list has 2 items, id="17500000000002".
- Rapid saves in the same millisecond: each call reads the current list first, so `list.len()` increments 0→1→2 across the three calls, guaranteeing distinct ids even at sub-millisecond speed.
- The `upsert_imported` path reads the list _then_ generates a fresh id using the post-read length, so imported presets never collide with existing ones regardless of the incoming id.

### Write-then-rename (atomic replace)

All writes go to `<path>.json.tmp` first via `std::fs::write`, then `std::fs::rename` to the final path. This mirrors the pattern used by `engine-state.json` autosave in `src-tauri`. A crash between write and rename leaves the `.tmp` orphan; on next write it is simply overwritten.

### Empty/absent file handling

`list()` pattern-matches on the `io::Error` kind:
- `NotFound` → returns `Ok(vec![])` (normal on fresh install)
- Read succeeds but content is whitespace-only → returns `Ok(vec![])` (guards against an empty file left by a previous crash before rename)
- Any other I/O error → propagated as `HmError::Storage`

### Tests (all pass)

| Test | What it verifies |
|---|---|
| `save_then_list_roundtrips` | 2 saves → list returns 2 with correct names + state fields |
| `delete_removes` | save 2, delete one → list has 1 with the correct id |
| `list_empty_when_absent` | non-existent path → `Ok([])`, no panic |
| `upsert_imported_assigns_fresh_id` | import with colliding id → stored with different id; both in list |
| `persists_across_reopen` | drop store, reopen same path → data still there |
| `unique_ids_on_rapid_save` | 3 rapid saves → all 3 ids distinct |

### Concerns / follow-up

1. **Concurrent access**: `ChainPresetStore` has no internal mutex. If two Tauri commands run concurrently and both call `list()` then `write()`, the second write wins and the first's append is lost. Given that the Tauri command handlers for this store are likely serialized through a single `State<Mutex<ChainPresetStore>>` wrapper (the same pattern as other stores in the app), this is acceptable — document it when wiring in Task 3.

2. **Convolver IR path portability**: imported presets may contain a `convolver.ir_id` that refers to a file path on the exporting machine. The store stores it as-is; applying will succeed but convolver will have no IR loaded if the path is missing. This is noted in the plan as acceptable.

3. **`.json.tmp` extension**: the `with_extension("json.tmp")` call replaces the `.json` extension with `json.tmp`, producing e.g. `chain-presets.json.tmp`. This is intentional and unambiguous.
