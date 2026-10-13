# src-tauri/src/commands/visualizer.rs

- PCM_FPS · constant · L23-L23 — const PCM_FPS: u64 = 30;
- VisualizerState · struct · L27-L29 — pub struct VisualizerState
- Running · struct · L31-L37 — struct Running
- shutdown · function · L40-L47 — fn shutdown(mut self)
- sidecar_path · function · L52-L71 — fn sidecar_path() -> Option<PathBuf>
- preset_dir · function · L75-L90 — fn preset_dir(app: &AppHandle) -> String
- visualizer_available · function · L94-L96 — pub fn visualizer_available() -> bool
- visualizer_preset_names · function · L102-L121 — pub fn visualizer_preset_names(app: AppHandle) -> Vec<String>
- visualizer_start · function · L126-L211 — pub fn visualizer_start(
- CREATE_NO_WINDOW · constant · L148-L148 — const CREATE_NO_WINDOW: u32 = 0x0800_0000;
- visualizer_set_preset · function · L216-L232 — pub fn visualizer_set_preset(state: State<'_, VisualizerState>, preset: String)
- visualizer_stop · function · L236-L240 — pub fn visualizer_stop(state: State<'_, VisualizerState>)
- visualizer_is_open · function · L244-L246 — pub fn visualizer_is_open(state: State<'_, VisualizerState>) -> bool
