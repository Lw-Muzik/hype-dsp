# crates/hm-dsp/src/gain.rs

- Gain · struct · L7-L9 — pub struct Gain
- default · function · L12-L14 — fn default() -> Self
- new · function · L19-L21 — pub fn new() -> Self
- prepare · function · L25-L25 — fn prepare(&mut self, _sample_rate: f32, _channels: usize) {}
- process · function · L27-L34 — fn process(&mut self, buffer: &mut [f32], _channels: usize)
- set_params · function · L36-L38 — fn set_params(&mut self, params: &ProcessorParams)
- tests · module · L42-L76 — mod tests
- applies_plus_six_db_as_linear_doubling · function · L47-L65 — fn applies_plus_six_db_as_linear_doubling()
- zero_db_is_identity · function · L68-L75 — fn zero_db_is_identity()
