# crates/hm-dsp/src/limiter.rs

- LOOKAHEAD_MS · constant · L13-L13 — const LOOKAHEAD_MS: f32 = 5.0;
- RELEASE_MS · constant · L15-L15 — const RELEASE_MS: f32 = 100.0;
- Limiter · struct · L18-L37 — pub struct Limiter
- new · function · L42-L58 — pub fn new(sample_rate: f32, channels: usize) -> Self
- reconfigure · function · L61-L76 — fn reconfigure(&mut self)
- prepare · function · L80-L84 — fn prepare(&mut self, sample_rate: f32, channels: usize)
- process · function · L86-L163 — fn process(&mut self, buffer: &mut [f32], channels: usize)
- set_params · function · L165-L168 — fn set_params(&mut self, params: &ProcessorParams)
- tests · module · L172-L382 — mod tests
- holds_output_below_ceiling · function · L178-L208 — fn holds_output_below_ceiling()
- NaiveLimiter · struct · L213-L220 — struct NaiveLimiter
- new · function · L223-L235 — fn new(sample_rate: f32, channels: usize, ceiling_db: f32) -> Self
- process · function · L237-L271 — fn process(&mut self, buffer: &mut [f32], channels: usize)
- assert_matches_naive · function · L277-L312 — fn assert_matches_naive(signal: &[f32], channels: usize, label: &str)
- matches_naive_on_impulse · function · L316-L322 — fn matches_naive_on_impulse()
- matches_naive_on_sine_burst · function · L327-L339 — fn matches_naive_on_sine_burst()
- matches_naive_on_noise · function · L344-L354 — fn matches_naive_on_noise()
- transparent_below_ceiling · function · L359-L381 — fn transparent_below_ceiling()
