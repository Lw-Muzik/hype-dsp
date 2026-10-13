# crates/hm-core/src/apo_ids.rs

- CLSID_STR · constant · L7-L7 — pub const CLSID_STR: &str = "{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";
- CLSID_GUID · constant · L10-L10 — pub const CLSID_GUID: Guid = Guid(0x7B1C4A20_9D3E_4E8A_9F2C_11AA22BB33CC);
- Guid · struct · L15-L15 — pub struct Guid(pub u128);
- to_u128 · function · L17-L19 — pub const fn to_u128(self) -> u128
- MAPPING_NAME · constant · L23-L23 — pub const MAPPING_NAME: &str = "Local\\HypeMuzikApoParams";
- CLSID_REGKEY · constant · L26-L27 — pub const CLSID_REGKEY: &str =
- APO_REGKEY · constant · L30-L31 — pub const APO_REGKEY: &str =
- DISABLE_PROTECTED_AUDIO_DG_KEY · constant · L34-L35 — pub const DISABLE_PROTECTED_AUDIO_DG_KEY: &str =
- DISABLE_PROTECTED_AUDIO_DG_VALUE · constant · L36-L36 — pub const DISABLE_PROTECTED_AUDIO_DG_VALUE: &str = "DisableProtectedAudioDG";
- FX_PROPERTIES_PKEY · constant · L41-L41 — pub const FX_PROPERTIES_PKEY: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";
- tests · module · L44-L62 — mod tests
- clsid_string_and_guid_agree · function · L47-L51 — fn clsid_string_and_guid_agree()
- registry_paths_are_hklm_relative_and_stable · function · L53-L61 — fn registry_paths_are_hklm_relative_and_stable()
