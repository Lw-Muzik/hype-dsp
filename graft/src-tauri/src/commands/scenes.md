# src-tauri/src/commands/scenes.rs

- SceneInfo · struct · L16-L21 — pub struct SceneInfo
- SCENES · constant · L25-L36 — pub const SCENES: &[SceneInfo] = &[
- DEFAULT_SCENE · constant · L38-L38 — const DEFAULT_SCENE: &str = "radial-spectrum";
- ScenePrefs · struct · L41-L43 — struct ScenePrefs
- default · function · L46-L48 — fn default() -> Self
- SceneState · struct · L52-L55 — pub struct SceneState
- load · function · L58-L67 — pub fn load(path: PathBuf) -> Self
- save · function · L69-L76 — fn save(&self, prefs: &ScenePrefs)
- scene_list · function · L81-L83 — pub fn scene_list() -> Vec<SceneInfo>
- scene_selected · function · L87-L89 — pub fn scene_selected(state: State<'_, SceneState>) -> String
- scene_select · function · L93-L100 — pub fn scene_select(state: State<'_, SceneState>, id: String)
