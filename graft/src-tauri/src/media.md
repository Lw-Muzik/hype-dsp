# src-tauri/src/media.rs

- MediaCommand · struct · L26-L31 — struct MediaCommand
- Update · enum · L34-L47 — enum Update
- MediaSession · struct · L52-L54 — pub struct MediaSession
- spawn · function · L58-L65 — pub fn spawn(app: AppHandle, hwnd: Option<isize>) -> Self
- set_metadata · function · L68-L83 — pub fn set_metadata(
- set_playback · function · L86-L92 — pub fn set_playback(&self, playing: bool, paused: bool, position_secs: f64)
- main_hwnd · function · L97-L102 — fn main_hwnd(app: &AppHandle) -> Option<isize>
- main_hwnd · function · L106-L108 — fn main_hwnd(_app: &AppHandle) -> Option<isize>
- start · function · L111-L114 — pub fn start(app: AppHandle) -> MediaSession
- run · function · L116-L184 — fn run(app: AppHandle, hwnd: Option<isize>, rx: Receiver<Update>)
