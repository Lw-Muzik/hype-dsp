# crates/hm-core/src/ddc_library.rs

- BundledDdc · struct · L13-L16 — struct BundledDdc
- DDC_JSON · constant · L20-L20 — const DDC_JSON: &str = include_str!("../data/ddc_presets.json");
- presets · function · L22-L27 — fn presets() -> &'static [BundledDdc]
- PRESETS · constant · L23-L23 — static PRESETS: OnceLock<Vec<BundledDdc>> = OnceLock::new();
- len · function · L30-L32 — pub fn len() -> usize
- is_empty · function · L35-L37 — pub fn is_empty() -> bool
- names · function · L40-L42 — pub fn names() -> Vec<String>
- get · function · L45-L50 — pub fn get(name: &str) -> Option<&'static str>
- tests · module · L53-L91 — mod tests
- library_is_substantial · function · L57-L60 — fn library_is_substantial()
- every_bundled_preset_resolves_to_finite_bands · function · L63-L76 — fn every_bundled_preset_resolves_to_finite_bands()
- unknown_name_is_none · function · L79-L81 — fn unknown_name_is_none()
- names_are_unique · function · L84-L90 — fn names_are_unique()
