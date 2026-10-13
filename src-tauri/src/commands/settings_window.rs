//! The Settings child window.
//!
//! Settings lives in its own native window (label `settings`) that is parented
//! to `main` and has only a close button: minimize and maximize are disabled,
//! and on macOS the miniaturize and zoom traffic lights are hidden. The window
//! loads the same `index.html` as `main`; the frontend branches on its label.

use hm_core::IpcError;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the Settings window (mirrored by `SETTINGS_WINDOW` in
/// `src/lib/windowLabel.ts`).
pub const LABEL: &str = "settings";

/// Section ids Settings can scroll to. Mirrors `SettingsSection` in
/// `src/lib/ipc.ts`; `src/lib/settingsSections.test.ts` pins the two together.
pub const SECTIONS: &[&str] = &[
    "about",
    "appearance",
    "library",
    "devices",
    "cloud",
    "youtube-music",
    "playback",
    "output",
    "account",
    "system-audio",
    "visualizer",
];

/// The section id if it is a known one; unknown ids are ignored.
fn known_section(section: Option<&str>) -> Option<&'static str> {
    let section = section?;
    SECTIONS.iter().copied().find(|s| *s == section)
}

/// Open the Settings window, or focus it (and scroll to `section`) if it exists.
pub fn open(app: &AppHandle, section: Option<&str>) -> tauri::Result<()> {
    let section = known_section(section);

    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        window.show()?;
        window.set_focus()?;
        if let Some(section) = section {
            let _ = app.emit_to(LABEL, "settings://section", section);
        }
        return Ok(());
    }

    let url = match section {
        Some(section) => format!("index.html#{section}"),
        None => "index.html".to_string(),
    };

    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title("Settings")
        .inner_size(820.0, 840.0)
        .min_inner_size(600.0, 480.0)
        .resizable(true)
        .minimizable(false)
        .maximizable(false)
        .theme(Some(tauri::Theme::Dark))
        .background_color(tauri::window::Color(0x0A, 0x0B, 0x0E, 0xFF))
        .center();

    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main)?;
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    let window = builder.build()?;

    #[cfg(not(target_os = "macos"))]
    {
        // The app menu attaches to every new window as a menu bar there.
        let _ = window.remove_menu();
    }

    #[cfg(target_os = "macos")]
    hide_zoom_buttons(&window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;

    Ok(())
}

/// Leave only the red close button: hide the miniaturize and zoom lights.
#[cfg(target_os = "macos")]
fn hide_zoom_buttons(window: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowButton};

    let Ok(ptr) = window.ns_window() else { return };
    let ptr = ptr as usize;
    let _ = window.run_on_main_thread(move || {
        // SAFETY: `ptr` is this window's live NSWindow, and AppKit calls run on
        // the main thread here.
        let ns_window = unsafe { &*(ptr as *const NSWindow) };
        for kind in [NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
            if let Some(button) = ns_window.standardWindowButton(kind) {
                button.setHidden(true);
            }
        }
    });
}

/// Close the Settings window if it is open (the main window hid or went away).
pub fn close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}

/// Open Settings, optionally scrolled to a section. Async: building a window
/// from a sync command deadlocks on Windows.
#[tauri::command]
pub async fn settings_open(app: AppHandle, section: Option<String>) -> Result<(), IpcError> {
    open(&app, section.as_deref()).map_err(|e| IpcError::new("settings_open", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_sections_only() {
        assert_eq!(known_section(Some("cloud")), Some("cloud"));
        assert_eq!(known_section(Some("nope")), None);
        assert_eq!(known_section(Some("cloud\"><script>")), None);
        assert_eq!(known_section(None), None);
    }

    #[test]
    fn section_ids_are_unique() {
        let mut ids = SECTIONS.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SECTIONS.len());
    }
}
