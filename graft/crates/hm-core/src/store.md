# crates/hm-core/src/store.rs

- PresetStore · struct · L18-L20 — pub struct PresetStore
- open · function · L25-L28 — pub fn open(path: &Path) -> Result<Self, HmError>
- open_in_memory · function · L31-L34 — pub fn open_in_memory() -> Result<Self, HmError>
- from_conn · function · L36-L59 — fn from_conn(conn: Connection) -> Result<Self, HmError>
- seed_builtins · function · L61-L86 — fn seed_builtins(&self) -> Result<(), HmError>
- list · function · L89-L108 — pub fn list(&self) -> Result<Vec<EqPreset>, HmError>
- get · function · L111-L128 — pub fn get(&self, id: &str) -> Result<EqPreset, HmError>
- save_custom · function · L131-L164 — pub fn save_custom(
- update · function · L167-L186 — pub fn update(&self, preset: &EqPreset) -> Result<(), HmError>
- delete · function · L189-L201 — pub fn delete(&self, id: &str) -> Result<(), HmError>
- to_preset · function · L204-L217 — fn to_preset(row: (String, String, i64, String, f64)) -> Result<EqPreset, HmError>
- tests · module · L220-L266 — mod tests
- seeds_builtins_on_open · function · L224-L232 — fn seeds_builtins_on_open()
- custom_preset_crud_roundtrip · function · L235-L257 — fn custom_preset_crud_roundtrip()
- builtins_cannot_be_deleted_or_edited · function · L260-L265 — fn builtins_cannot_be_deleted_or_edited()
