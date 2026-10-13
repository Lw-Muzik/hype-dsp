# crates/hm-dsp/src/script_stage.rs

- ScriptSlot · type · L38-L38 — pub type ScriptSlot = Arc<ArcSwap<Option<Arc<Program>>>>;
- empty_script_slot · function · L41-L43 — pub fn empty_script_slot() -> ScriptSlot
- BLOCK_BUDGET · constant · L60-L60 — const BLOCK_BUDGET: u32 = 2_000_000;
- ScriptProcessor · struct · L63-L75 — pub struct ScriptProcessor
- new · function · L79-L81 — pub fn new(sample_rate: f32, channels: usize) -> Self
- with_slot · function · L85-L93 — pub fn with_slot(sample_rate: f32, _channels: usize, slot: ScriptSlot) -> Self
- slot · function · L96-L98 — pub fn slot(&self) -> ScriptSlot
- flush · function · L104-L106 — fn flush(x: f32) -> f32
- prepare · function · L109-L117 — fn prepare(&mut self, sample_rate: f32, _channels: usize)
- process · function · L119-L179 — fn process(&mut self, buffer: &mut [f32], channels: usize)
- set_params · function · L181-L183 — fn set_params(&mut self, params: &ProcessorParams)
- tests · module · L191-L320 — mod tests
- params_with_script · function · L197-L202 — fn params_with_script(enabled: bool) -> EngineState
- disabled_is_identity · function · L209-L218 — fn disabled_is_identity()
- no_program_is_identity · function · L225-L233 — fn no_program_is_identity()
- gain_program_halves · function · L240-L261 — fn gain_program_halves()
- stays_bounded · function · L268-L288 — fn stays_bounded()
- program_change_detected · function · L295-L319 — fn program_change_detected()
