# crates/hm-dsp/src/graphic_eq.rs

- DEFAULT_Q · constant · L9-L9 — pub const DEFAULT_Q: f32 = 4.318;
- GraphicEq · struct · L13-L22 — pub struct GraphicEq
- new · function · L26-L38 — pub fn new(sample_rate: f32, channels: usize) -> Self
- retune · function · L42-L53 — fn retune(&mut self)
- prepare · function · L57-L62 — fn prepare(&mut self, sample_rate: f32, channels: usize)
- process · function · L64-L79 — fn process(&mut self, buffer: &mut [f32], channels: usize)
- set_params · function · L81-L88 — fn set_params(&mut self, params: &ProcessorParams)
- tests · module · L92-L163 — mod tests
- flat_params · function · L97-L99 — fn flat_params() -> EngineState
- flat_eq_is_identity · function · L103-L114 — fn flat_eq_is_identity()
- response_db_at · function · L117-L132 — fn response_db_at(eq: &mut GraphicEq, sample_rate: f32, freq_hz: f32) -> f32
- N · constant · L118-L118 — const N: usize = 16_384;
- single_band_boost_is_localized · function · L137-L162 — fn single_band_boost_is_localized()
