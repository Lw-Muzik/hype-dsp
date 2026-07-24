//! One-click setup for the free Windows APO system-wide EQ backend.
//!
//! Installing the APO writes to HKLM and copies a DLL into ProgramFiles, so it
//! needs elevation. Like the VB-CABLE flow, we get one UAC prompt by
//! re-launching ourselves elevated with `--apo-install <dll>` (handled early in
//! `lib.rs::run`); the actual registry/endpoint work lives in
//! `hm_audio::system_eq_windows_apo`. Cross-platform command signatures (the
//! non-Windows arms return an error) so the handler registers on every OS.

use hm_core::IpcError;

/// How an APO setup attempt ended.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ApoSetupOutcome {
    /// Installed + attached; a reboot finalizes it for the audio engine.
    NeedsReboot,
}

/// Install the APO (copy DLL, register, set `DisableProtectedAudioDG`, attach to
/// the default endpoint) behind one UAC prompt, then report the outcome.
#[tauri::command]
pub fn apo_setup(app: tauri::AppHandle) -> Result<ApoSetupOutcome, IpcError> {
    #[cfg(target_os = "windows")]
    {
        imp::setup(&app).map_err(|e| IpcError::new("apo_setup", e))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err(IpcError::new(
            "unsupported",
            "the APO backend is Windows-only",
        ))
    }
}

/// Remove the APO (detach, unregister, clear the global flag) behind one UAC prompt.
#[tauri::command]
pub fn apo_uninstall(app: tauri::AppHandle) -> Result<(), IpcError> {
    #[cfg(target_os = "windows")]
    {
        imp::uninstall(&app).map_err(|e| IpcError::new("apo_uninstall", e))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Ok(())
    }
}

/// Re-attach the APO to the current default endpoint if a Windows update or a
/// device change detached it. Unelevated best-effort (registry write may need
/// elevation on locked-down machines).
#[tauri::command]
pub fn apo_repair() -> Result<(), IpcError> {
    #[cfg(target_os = "windows")]
    {
        hm_audio::system_eq_windows_apo::repair().map_err(|e| IpcError::new("apo_repair", e.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use tauri::Manager;

    use super::ApoSetupOutcome;

    /// `CREATE_NO_WINDOW` — this is a GUI app, so spawned console helpers must not
    /// flash a terminal.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn setup(app: &tauri::AppHandle) -> Result<ApoSetupOutcome, String> {
        let dll = app
            .path()
            .resource_dir()
            .map_err(|e| format!("resource dir: {e}"))?
            .join("apo")
            .join("hm_apo.dll");
        if !dll.exists() {
            return Err(format!(
                "the APO DLL is not bundled in this build ({}). See docs/windows-apo.md.",
                dll.display()
            ));
        }
        run_elevated(&["--apo-install", &dll.to_string_lossy()])?;
        if hm_audio::system_eq_windows_apo::apo_installed() {
            Ok(ApoSetupOutcome::NeedsReboot)
        } else {
            Err("the APO did not register (install was cancelled or failed)".into())
        }
    }

    pub fn uninstall(_app: &tauri::AppHandle) -> Result<(), String> {
        run_elevated(&["--apo-uninstall"])
    }

    /// Re-launch this executable elevated with `args` and wait for it (one UAC
    /// prompt). The elevated instance handles the flag in `lib.rs::run` and exits.
    fn run_elevated(args: &[&str]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
        let arg_list = args
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let ps = format!(
            "Start-Process -FilePath '{}' -ArgumentList {} -Verb RunAs -Wait",
            exe.to_string_lossy().replace('\'', "''"),
            arg_list
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("elevation failed: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("the elevation prompt was declined".into())
        }
    }
}
