//! Opening Settings from native entry points (app menu, tray).
//!
//! Settings is an in-app modal rendered by the main window's frontend, so the
//! backend only has to bring that window forward and tell it to open.

use tauri::{AppHandle, Emitter, Manager};

/// Show and focus the main window, then ask its frontend to open Settings.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit_to("main", "settings://open", ());
}
