# crates/hm-core/src/chain_presets.rs

- ChainPreset · struct · L28-L35 — pub struct ChainPreset
- ChainPresetStore · struct · L44-L46 — pub struct ChainPresetStore
- open · function · L52-L56 — pub fn open(path: &Path) -> Self
- list · function · L62-L72 — pub fn list(&self) -> Result<Vec<ChainPreset>, HmError>
- save · function · L77-L92 — pub fn save(&self, name: &str, state: EngineState) -> Result<ChainPreset, HmError>
- delete · function · L97-L105 — pub fn delete(&self, id: &str) -> Result<(), HmError>
- upsert_imported · function · L111-L122 — pub fn upsert_imported(&self, preset: ChainPreset) -> Result<ChainPreset, HmError>
- write · function · L130-L144 — fn write(&self, list: &[ChainPreset]) -> Result<(), HmError>
- unique_id · function · L152-L158 — fn unique_id(list_len: usize) -> String
- tests · module · L163-L301 — mod tests
- temp_path · function · L168-L170 — fn temp_path(name: &str) -> PathBuf
- cleanup · function · L172-L175 — fn cleanup(path: &Path)
- save_then_list_roundtrips · function · L178-L200 — fn save_then_list_roundtrips()
- delete_removes · function · L203-L219 — fn delete_removes()
- list_empty_when_absent · function · L222-L229 — fn list_empty_when_absent()
- upsert_imported_assigns_fresh_id · function · L232-L260 — fn upsert_imported_assigns_fresh_id()
- persists_across_reopen · function · L263-L279 — fn persists_across_reopen()
- unique_ids_on_rapid_save · function · L282-L300 — fn unique_ids_on_rapid_save()
