//! Licensing commands (backed by the local mock — see `hm_core::LicenseMock`).

use hm_core::{IpcError, LicenseMock, LicenseService, LicenseStatus};
use tauri::State;

#[tauri::command]
pub fn license_status(license: State<'_, LicenseMock>) -> LicenseStatus {
    license.status()
}

#[tauri::command]
pub fn license_activate(
    license: State<'_, LicenseMock>,
    key: String,
) -> Result<LicenseStatus, IpcError> {
    license
        .activate(&key)
        .map_err(|e| IpcError::new("license", e.to_string()))
}

#[tauri::command]
pub fn license_deactivate(license: State<'_, LicenseMock>) -> Result<(), IpcError> {
    license
        .deactivate()
        .map_err(|e| IpcError::new("license", e.to_string()))
}
