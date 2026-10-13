# crates/hm-core/src/vdc_import.rs

- VdcCurve · struct · L27-L30 — pub struct VdcCurve
- rate_rank · function · L35-L43 — fn rate_rank(rate: f64) -> u8
- parse_vdc · function · L50-L109 — pub fn parse_vdc(input: &str) -> Result<VdcCurve, String>
- cascade_response_db · function · L113-L140 — fn cascade_response_db(biquads: &[[f64; 5]], sample_rate: f64, freq_hz: f64) -> f64
- vdc_to_iso_bands · function · L145-L154 — pub fn vdc_to_iso_bands(input: &str) -> Result<[f32; BAND_COUNT], String>
- tests · module · L157-L223 — mod tests
- identity_biquad_is_flat_zero_db · function · L161-L168 — fn identity_biquad_is_flat_zero_db()
- pure_gain_biquad_is_flat_gain · function · L171-L179 — fn pure_gain_biquad_is_flat_gain()
- prefers_44100_then_48000 · function · L182-L187 — fn prefers_44100_then_48000()
- falls_back_to_first_when_no_preferred_rate · function · L190-L193 — fn falls_back_to_first_when_no_preferred_rate()
- rejects_malformed · function · L196-L201 — fn rejects_malformed()
- parses_scientific_notation_and_multiple_sections · function · L204-L209 — fn parses_scientific_notation_and_multiple_sections()
- real_world_section_yields_finite_bounded_response · function · L212-L222 — fn real_world_section_yields_finite_bounded_response()
