//! Tauri command handlers — the typed IPC surface the React app calls.
//!
//! App-defined commands are always callable from the front end (the ACL in
//! `capabilities/` governs only core/plugin commands). Each command returns a
//! `serde`-serializable value or `hm_core::IpcError`, mirrored in
//! `src/lib/ipc.ts`.

pub mod account;
pub mod apo_setup;
pub mod app;
pub mod audio;
pub mod autoeq;
pub mod cable;
pub mod chain_presets;
pub mod cloud;
pub mod engine;
pub mod identify;
pub mod library;
pub mod link;
pub mod license;
pub mod linux_audio_setup;
pub mod lyrics;
pub mod mixer;
pub mod open_with;
pub mod presets;
pub mod profiles;
pub mod radio;
pub mod scenes;
pub mod stems;
pub mod tv;
pub mod visualizer;
pub mod ytmusic;
pub mod ytmusic_setup;
