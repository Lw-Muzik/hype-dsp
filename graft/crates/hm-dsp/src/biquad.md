# crates/hm-dsp/src/biquad.rs

- Biquad · struct · L8-L16 — pub struct Biquad
- default · function · L19-L21 — fn default() -> Self
- identity · function · L26-L36 — pub fn identity() -> Self
- set_peaking · function · L41-L63 — pub fn set_peaking(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32)
- set_low_shelf · function · L66-L83 — pub fn set_low_shelf(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32)
- set_high_shelf · function · L86-L103 — pub fn set_high_shelf(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32)
- set_lowpass · function · L106-L120 — pub fn set_lowpass(&mut self, sample_rate: f32, f0: f32, q: f32)
- set_highpass · function · L123-L137 — pub fn set_highpass(&mut self, sample_rate: f32, f0: f32, q: f32)
- set_from_kind · function · L141-L147 — pub fn set_from_kind(&mut self, kind: &str, sample_rate: f32, f0: f32, gain_db: f32, q: f32)
- assign · function · L149-L155 — fn assign(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64)
- reset · function · L158-L161 — pub fn reset(&mut self)
- process_sample · function · L165-L170 — pub fn process_sample(&mut self, x: f32) -> f32
- tests · module · L174-L241 — mod tests
- peaking_at_zero_db_is_identity · function · L181-L190 — fn peaking_at_zero_db_is_identity()
- low_shelf_at_zero_db_is_identity · function · L193-L200 — fn low_shelf_at_zero_db_is_identity()
- low_shelf_boost_raises_dc_gain · function · L203-L213 — fn low_shelf_boost_raises_dc_gain()
- lowpass_passes_dc_attenuates_hf · function · L216-L231 — fn lowpass_passes_dc_attenuates_hf()
- highpass_blocks_dc_passes_hf · function · L234-L240 — fn highpass_blocks_dc_passes_hf()
