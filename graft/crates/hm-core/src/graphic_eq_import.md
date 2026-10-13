# crates/hm-core/src/graphic_eq_import.rs

- parse_graphic_eq · function · L10-L41 — pub fn parse_graphic_eq(input: &str) -> Result<Vec<(f32, f32)>, String>
- interpolate_to_iso_bands · function · L45-L77 — pub fn interpolate_to_iso_bands(curve: &[(f32, f32)]) -> [f32; BAND_COUNT]
- recommended_preamp · function · L80-L83 — pub fn recommended_preamp(bands: &[f32; BAND_COUNT]) -> f32
- tests · module · L86-L136 — mod tests
- parses_labeled_and_unlabeled · function · L90-L96 — fn parses_labeled_and_unlabeled()
- rejects_malformed · function · L99-L103 — fn rejects_malformed()
- interpolation_hits_exact_points · function · L106-L112 — fn interpolation_hits_exact_points()
- preamp_is_clip_proof · function · L115-L121 — fn preamp_is_clip_proof()
- preamp_zero_when_no_positive_gain · function · L124-L128 — fn preamp_zero_when_no_positive_gain()
- single_point_curve_clamps_all_bands · function · L131-L135 — fn single_point_curve_clamps_all_bands()
