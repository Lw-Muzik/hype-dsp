# src-tauri/src/commands/mixer.rs

- Mixer · type · L12-L12 — pub type Mixer = Mutex<Box<dyn SessionController>>;
- MixerSnapshot · struct · L17-L21 — pub struct MixerSnapshot
- mixer_list_sessions · function · L29-L38 — pub fn mixer_list_sessions(mixer: State<'_, Mixer>) -> MixerSnapshot
- mixer_set_volume · function · L47-L53 — pub fn mixer_set_volume(mixer: State<'_, Mixer>, id: String, gain: f32) -> Result<(), IpcError>
- mixer_set_muted · function · L56-L62 — pub fn mixer_set_muted(mixer: State<'_, Mixer>, id: String, muted: bool) -> Result<(), IpcError>
