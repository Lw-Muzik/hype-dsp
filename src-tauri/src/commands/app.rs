//! Application metadata commands.

use hm_core::AppInfo;

/// Return the product name, version, and engine schema revision.
///
/// This is the canonical Phase 0 round-trip: it proves the typed IPC seam works
/// end to end (Rust `AppInfo` → JSON → TS `AppInfo`). It cannot fail.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo::current("HypeMuzik", env!("CARGO_PKG_VERSION"))
}
