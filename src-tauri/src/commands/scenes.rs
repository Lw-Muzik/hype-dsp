//! In-app (Canvas/WebGL) visualizer scenes.
//!
//! A backend-owned registry of the available web visualizers plus which one is
//! selected (persisted to disk). The frontend maps each id to a renderer, but
//! the backend is the source of truth for the list and the current selection so
//! it survives restarts — the same way the rest of the app's state is managed.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

/// One selectable in-app visualizer.
#[derive(Clone, Serialize)]
pub struct SceneInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// "2d" (Canvas) or "3d" (WebGL / Three.js).
    pub kind: &'static str,
}

/// The full registry (source of truth). The frontend renders the ones it has a
/// component for and shows the rest as "coming soon".
pub const SCENES: &[SceneInfo] = &[
    SceneInfo { id: "radial-spectrum", name: "Radial Spectrum", kind: "2d" },
    SceneInfo { id: "neon-bars", name: "Neon Bars", kind: "2d" },
    SceneInfo { id: "oscilloscope", name: "Oscilloscope", kind: "2d" },
    SceneInfo { id: "particle-burst", name: "Particle Burst", kind: "2d" },
    SceneInfo { id: "liquid-blob", name: "Liquid Blob", kind: "2d" },
    SceneInfo { id: "audio-sphere", name: "Audio Sphere", kind: "3d" },
    SceneInfo { id: "particle-galaxy", name: "Particle Galaxy", kind: "3d" },
    SceneInfo { id: "audio-terrain", name: "Audio Terrain", kind: "3d" },
    SceneInfo { id: "tunnel", name: "Tunnel", kind: "3d" },
    SceneInfo { id: "eq-city", name: "Equalizer City", kind: "3d" },
];

const DEFAULT_SCENE: &str = "radial-spectrum";

#[derive(Serialize, Deserialize)]
struct ScenePrefs {
    selected: String,
}

impl Default for ScenePrefs {
    fn default() -> Self {
        Self { selected: DEFAULT_SCENE.to_owned() }
    }
}

/// Managed Tauri state: the selected scene + its on-disk path.
pub struct SceneState {
    inner: Mutex<ScenePrefs>,
    path: PathBuf,
}

impl SceneState {
    pub fn load(path: PathBuf) -> Self {
        let prefs = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<ScenePrefs>(&t).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(prefs),
            path,
        }
    }

    fn save(&self, prefs: &ScenePrefs) {
        if let Ok(json) = serde_json::to_string_pretty(prefs) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

/// The available in-app visualizer scenes (registry).
#[tauri::command]
pub fn scene_list() -> Vec<SceneInfo> {
    SCENES.to_vec()
}

/// The currently selected scene id.
#[tauri::command]
pub fn scene_selected(state: State<'_, SceneState>) -> String {
    state.inner.lock().expect("scenes poisoned").selected.clone()
}

/// Select a scene (validated against the registry) and persist it.
#[tauri::command]
pub fn scene_select(state: State<'_, SceneState>, id: String) {
    if !SCENES.iter().any(|s| s.id == id) {
        return;
    }
    let mut prefs = state.inner.lock().expect("scenes poisoned");
    prefs.selected = id;
    state.save(&prefs);
}
