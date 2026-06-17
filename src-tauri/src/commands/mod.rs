//! Tauri command handlers — the typed IPC surface the React app calls.
//!
//! App-defined commands are always callable from the front end (the ACL in
//! `capabilities/` governs only core/plugin commands). Each command returns a
//! `serde`-serializable value or `hm_core::IpcError`, mirrored in
//! `src/lib/ipc.ts`.

pub mod app;
pub mod audio;
pub mod engine;
pub mod presets;
pub mod profiles;
