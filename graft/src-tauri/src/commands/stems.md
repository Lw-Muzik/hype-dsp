# src-tauri/src/commands/stems.rs

- StemState · struct · L23-L25 — pub struct StemState
- InFlight · struct · L33-L36 — struct InFlight
- in_flight · function · L38-L41 — fn in_flight() -> &'static Mutex<Option<InFlight>>
- IN_FLIGHT · constant · L39-L39 — static IN_FLIGHT: OnceLock<Mutex<Option<InFlight>>> = OnceLock::new();
- arm_in_flight · function · L46-L62 — fn arm_in_flight(track: &str) -> Arc<AtomicBool>
- finish_in_flight · function · L66-L74 — fn finish_in_flight(cancel: &Arc<AtomicBool>)
- cancel_in_flight · function · L77-L85 — fn cancel_in_flight()
- StemStatus · struct · L89-L96 — pub struct StemStatus
- stems_status · function · L99-L106 — pub fn stems_status(stems: State<'_, StemState>, track_path: String) -> StemStatus
- stems_arm · function · L112-L166 — pub fn stems_arm(
- stems_set_gain · function · L171-L173 — pub fn stems_set_gain(engine: State<'_, AudioEngine>, stem: usize, gain: f32)
- stems_reset · function · L178-L185 — pub fn stems_reset(engine: State<'_, AudioEngine>)
- stems_gains · function · L189-L192 — pub fn stems_gains(engine: State<'_, AudioEngine>) -> Vec<f32>
