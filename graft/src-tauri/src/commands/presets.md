# src-tauri/src/commands/presets.rs

- to_band_array · function · L7-L11 — fn to_band_array(bands: Vec<f32>) -> Result<[f32; BAND_COUNT], IpcError>
- eq_list_presets · function · L15-L17 — pub fn eq_list_presets(store: State<'_, PresetStore>) -> Result<Vec<EqPreset>, IpcError>
- eq_apply_preset · function · L21-L29 — pub fn eq_apply_preset(
- eq_save_custom · function · L33-L43 — pub fn eq_save_custom(
- eq_update · function · L47-L49 — pub fn eq_update(store: State<'_, PresetStore>, preset: EqPreset) -> Result<(), IpcError>
- eq_delete · function · L53-L55 — pub fn eq_delete(store: State<'_, PresetStore>, id: String) -> Result<(), IpcError>
