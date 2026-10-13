//! The system tray (macOS menu bar) icon: a shortcut menu for most of the app.
//!
//! The main window's frontend is the source of truth. It pushes a [`TrayModel`]
//! with `tray_update` whenever the state the menu shows changes, and the menu
//! is rebuilt from it. Clicks on items the frontend owns (playback, mode,
//! effects, presets, navigation…) are sent back as `tray://action` with the
//! item id, and run through the same store actions the UI uses, so premium
//! gates and the mode switch behave exactly as they do in the window. Only
//! window-level items — Show, Settings, Quit — are handled here.

use std::sync::Mutex;

use hm_core::IpcError;
use serde::Deserialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

/// The tray icon's id.
const TRAY_ID: &str = "main";
/// Menu clicks the frontend handles are sent to the main window on this event.
pub const ACTION_EVENT: &str = "tray://action";

/// What the menu shows. Mirrors `TrayModel` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayModel {
    pub now_playing: Option<String>,
    pub playing: bool,
    pub can_control: bool,
    pub mode: String,
    pub mode_supported: bool,
    pub power: bool,
    pub effects: Vec<TrayEffect>,
    pub presets: Vec<TrayPreset>,
    pub routes: Vec<TrayRoute>,
    pub visualizer: Option<bool>,
    pub update_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayEffect {
    pub id: String,
    pub label: String,
    pub on: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayPreset {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayRoute {
    pub id: String,
    pub label: String,
}

/// The model the menu was last built from, so an unchanged push is a no-op.
#[derive(Default)]
pub struct TrayState(Mutex<Option<TrayModel>>);

/// Create the tray icon with a starting menu (the frontend replaces it as soon
/// as it has pushed its first model). A failure is logged, never fatal: on a
/// Linux desktop without an AppIndicator host there is simply no tray.
pub fn init(app: &AppHandle) {
    if let Err(e) = try_init(app) {
        eprintln!("tray: could not create the tray icon: {e}");
    }
}

fn try_init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &TrayModel { mode: "player".into(), mode_supported: true, ..Default::default() })?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("HypeMuzik")
        .menu(&menu)
        // macOS convention: a menu-bar extra opens its menu on click. Windows and
        // Linux: left click brings the window back, right click opens the menu.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| on_menu_item(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if cfg!(target_os = "macos") {
                return;
            }
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });

    #[cfg(target_os = "macos")]
    {
        // A monochrome template image, so macOS tints it for light and dark menu bars.
        builder = builder
            .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
            .icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
    }

    builder.build(app)?;
    Ok(())
}

/// Rebuild the menu from the frontend's model (a no-op when nothing changed).
#[tauri::command(async)]
pub fn tray_update(app: AppHandle, state: State<'_, TrayState>, model: TrayModel) -> Result<(), IpcError> {
    let fail = |e: tauri::Error| IpcError::new("tray", e.to_string());
    {
        let mut last = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(&model) {
            return Ok(());
        }
        *last = Some(model.clone());
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(()); // no tray on this desktop
    };
    let menu = build_menu(&app, &model).map_err(fail)?;
    tray.set_menu(Some(menu)).map_err(fail)?;
    tray.set_tooltip(Some(tooltip(&model))).map_err(fail)?;
    Ok(())
}

fn tooltip(model: &TrayModel) -> String {
    match (model.mode.as_str(), &model.now_playing) {
        ("equalizer", _) => "HypeMuzik — equalizing system audio".to_string(),
        (_, Some(title)) => format!("HypeMuzik — {title}"),
        _ => "HypeMuzik".to_string(),
    }
}

/// Route a click: window-level items here, everything else to the frontend.
fn on_menu_item(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
        "settings" => {
            // Off the event-loop thread: building a window from it can deadlock
            // on Windows (see `settings_window`).
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::settings_window::open(&app, None);
            });
        }
        "quit" => app.exit(0),
        _ => {
            // Navigation needs the window on screen to mean anything.
            if id.starts_with("go:") || id == "presets:open" {
                show_main(app);
            }
            let _ = app.emit_to("main", ACTION_EVENT, id);
        }
    }
}

/// Bring the main window back (it may be hidden by close, minimized, or the
/// whole app hidden with ⌘H).
fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The menu for `model`. Layout:
///
/// ```text
/// ♪ Now playing (or the Equalizer-mode status)
/// Play/Pause · Next · Previous          (Player mode)
/// ───
/// Player mode / Equalizer mode · Enhancement on
/// Equalizer preset ▸ · Effects ▸ · Fullscreen visualizer
/// ───
/// Go to ▸ · Settings…
/// ───
/// Restart to update (x.y.z) · Show HypeMuzik · Quit HypeMuzik
/// ```
fn build_menu(app: &AppHandle, model: &TrayModel) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let sep = || PredefinedMenuItem::separator(app);
    let equalizer_mode = model.mode == "equalizer";

    // Status line + transport.
    let status = if equalizer_mode {
        "Equalizing system audio".to_string()
    } else {
        match &model.now_playing {
            Some(title) => format!("♪ {}", ellipsize(title, 48)),
            None => "Nothing playing".to_string(),
        }
    };
    menu.append(&MenuItem::with_id(app, "status", status, false, None::<&str>)?)?;
    if !equalizer_mode {
        let play_label = if model.playing { "Pause" } else { "Play" };
        menu.append(&MenuItem::with_id(app, "play_pause", play_label, model.can_control, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "next", "Next", model.can_control, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "prev", "Previous", model.can_control, None::<&str>)?)?;
    }
    menu.append(&sep()?)?;

    // Mode, power, tuning.
    menu.append(&CheckMenuItem::with_id(app, "mode:player", "Player mode", true, !equalizer_mode, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        "mode:equalizer",
        "Equalizer mode",
        model.mode_supported,
        equalizer_mode,
        None::<&str>,
    )?)?;
    menu.append(&CheckMenuItem::with_id(app, "power", "Enhancement on", true, model.power, None::<&str>)?)?;

    let presets = Submenu::with_id(app, "presets", "Equalizer preset", true)?;
    for p in &model.presets {
        presets.append(&CheckMenuItem::with_id(app, format!("preset:{}", p.id), &p.name, true, p.active, None::<&str>)?)?;
    }
    if !model.presets.is_empty() {
        presets.append(&sep()?)?;
    }
    presets.append(&MenuItem::with_id(app, "presets:open", "Open Equalizer…", true, None::<&str>)?)?;
    menu.append(&presets)?;

    let effects = Submenu::with_id(app, "effects", "Effects", !model.effects.is_empty())?;
    for fx in &model.effects {
        effects.append(&CheckMenuItem::with_id(app, format!("fx:{}", fx.id), &fx.label, fx.enabled, fx.on, None::<&str>)?)?;
    }
    menu.append(&effects)?;

    if let Some(running) = model.visualizer {
        menu.append(&CheckMenuItem::with_id(app, "visualizer", "Fullscreen visualizer", true, running, None::<&str>)?)?;
    }
    menu.append(&sep()?)?;

    // Navigation.
    let go = Submenu::with_id(app, "go", "Go to", !model.routes.is_empty())?;
    for r in &model.routes {
        go.append(&MenuItem::with_id(app, format!("go:{}", r.id), &r.label, true, None::<&str>)?)?;
    }
    menu.append(&go)?;
    menu.append(&MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?)?;
    menu.append(&sep()?)?;

    // App.
    if let Some(version) = &model.update_version {
        menu.append(&MenuItem::with_id(
            app,
            "update_restart",
            format!("Restart to update ({version})"),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&MenuItem::with_id(app, "show", "Show HypeMuzik", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit HypeMuzik", true, None::<&str>)?)?;
    Ok(menu)
}

/// `s` cut to at most `max` characters, with an ellipsis when cut.
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_the_frontend_model() {
        let json = r#"{
            "nowPlaying": "Song", "playing": true, "canControl": true,
            "mode": "player", "modeSupported": true, "power": true,
            "effects": [{"id": "bass", "label": "Bass boost", "on": true, "enabled": true}],
            "presets": [{"id": "rock", "name": "Rock", "active": false}],
            "routes": [{"id": "player", "label": "Library"}],
            "visualizer": null, "updateVersion": "0.1.30"
        }"#;
        let m: TrayModel = serde_json::from_str(json).unwrap();
        assert_eq!(m.now_playing.as_deref(), Some("Song"));
        assert_eq!(m.effects[0].id, "bass");
        assert_eq!(m.visualizer, None);
        assert_eq!(m.update_version.as_deref(), Some("0.1.30"));
    }

    #[test]
    fn long_titles_are_cut_with_an_ellipsis() {
        assert_eq!(ellipsize("short", 48), "short");
        let long = "a".repeat(60);
        let cut = ellipsize(&long, 48);
        assert_eq!(cut.chars().count(), 48);
        assert!(cut.ends_with('…'));
        // Multi-byte characters are counted, not bytes.
        assert_eq!(ellipsize("ééééé", 3), "éé…");
    }

    #[test]
    fn tooltip_names_the_track_or_the_mode() {
        let mut m = TrayModel { mode: "player".into(), now_playing: Some("Song".into()), ..Default::default() };
        assert_eq!(tooltip(&m), "HypeMuzik — Song");
        m.mode = "equalizer".into();
        assert_eq!(tooltip(&m), "HypeMuzik — equalizing system audio");
        m.mode = "player".into();
        m.now_playing = None;
        assert_eq!(tooltip(&m), "HypeMuzik");
    }
}
