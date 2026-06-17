//! HypeMuzik desktop — Tauri application entry point.
//!
//! Wires the internal crates (`hm-*`) to the webview UI: registers plugins and
//! the typed command handlers, then runs the event loop. The heavy lifting
//! (DSP, audio engine, media, persistence) lives in the workspace crates; this
//! layer is the thin, well-documented bridge between Rust and React.

mod commands;

/// Build and run the Tauri application.
///
/// `run` is invoked from `main.rs` (desktop) and is the mobile entry point too.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_list_output_devices,
            commands::audio::audio_list_input_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the HypeMuzik application");
}
