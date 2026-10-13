# crates/hm-audio/src/resampler.rs

- StereoResampler · struct · L10-L17 — pub struct StereoResampler
- default · function · L20-L28 — fn default() -> Self
- new · function · L32-L34 — pub fn new() -> Self
- set_ratio · function · L38-L42 — pub fn set_ratio(&mut self, in_rate: u32, out_rate: u32)
- next_frame · function · L48-L82 — pub fn next_frame<F>(&mut self, mut pull: F) -> Option<(f32, f32)>
- tests · module · L86-L165 — mod tests
- resamples_ramp_at_correct_rate · function · L92-L120 — fn resamples_ramp_at_correct_rate()
- unity_ratio_is_passthrough · function · L124-L140 — fn unity_ratio_is_passthrough()
- consumes_input_at_capture_rate · function · L144-L164 — fn consumes_input_at_capture_rate()
