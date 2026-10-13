# crates/hm-core/src/presets.rs

- interp_log · function · L10-L27 — fn interp_log(anchors: &[(f32, f32)], freq: f32) -> f32
- curve · function · L29-L35 — fn curve(anchors: &[(f32, f32)]) -> [f32; BAND_COUNT]
- preset · function · L37-L45 — fn preset(id: &str, name: &str, anchors: &[(f32, f32)]) -> EqPreset
- builtins · function · L48-L172 — pub fn builtins() -> Vec<EqPreset>
- tests · module · L175-L195 — mod tests
- flat_preset_is_all_zero · function · L179-L183 — fn flat_preset_is_all_zero()
- bass_boost_lifts_lows_not_highs · function · L186-L194 — fn bass_boost_lifts_lows_not_highs()
