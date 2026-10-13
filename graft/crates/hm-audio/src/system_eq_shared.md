# crates/hm-audio/src/system_eq_shared.rs

- process_block · function · L34-L49 — pub(crate) fn process_block(
- tests · module · L52-L165 — mod tests
- RATE · constant · L55-L55 — const RATE: f32 = 48_000.0;
- CHANNELS · constant · L56-L56 — const CHANNELS: usize = 2;
- flat_state · function · L63-L74 — fn flat_state() -> EngineState
- power_off_is_identity · function · L77-L90 — fn power_off_is_identity()
- master_volume_scales_when_bypassed · function · L93-L110 — fn master_volume_scales_when_bypassed()
- unity_volume_flat_eq_is_near_identity · function · L113-L128 — fn unity_volume_flat_eq_is_near_identity()
- enabled_eq_changes_signal_and_stays_finite · function · L131-L149 — fn enabled_eq_changes_signal_and_stays_finite()
- sample_signal · function · L153-L164 — fn sample_signal() -> Vec<f32>
