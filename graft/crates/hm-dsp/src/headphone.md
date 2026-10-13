# crates/hm-dsp/src/headphone.rs

- MAX_BANDS · constant · L13-L13 — const MAX_BANDS: usize = 24;
- HeadphoneCorrection · struct · L15-L23 — pub struct HeadphoneCorrection
- new · function · L26-L38 — pub fn new(sample_rate: f32, channels: usize) -> Self
- reconfigure · function · L40-L43 — fn reconfigure(&mut self)
- retune · function · L45-L57 — fn retune(&mut self)
- prepare · function · L61-L65 — fn prepare(&mut self, sample_rate: f32, channels: usize)
- process · function · L67-L82 — fn process(&mut self, buffer: &mut [f32], channels: usize)
- set_params · function · L84-L92 — fn set_params(&mut self, params: &ProcessorParams)
- tests · module · L96-L139 — mod tests
- disabled_is_identity · function · L101-L108 — fn disabled_is_identity()
- applies_parametric_band · function · L111-L138 — fn applies_parametric_band()
