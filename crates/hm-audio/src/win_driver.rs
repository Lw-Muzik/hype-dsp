//! Windows bundled-driver lifecycle for system-wide EQ (the "Option 2" model).
//!
//! System-wide EQ on Windows routes every app's audio through a **virtual output
//! device** (`system_eq_windows`), captures it, runs the DSP chain, and renders to
//! the real device. For a zero-setup experience — the Boom3D / FxSound model — we
//! ship a **bundled, signed virtual audio driver** that provides that device, so
//! the user never has to install a third-party cable.
//!
//! This module installs and reports the status of that driver. Two things to know:
//!
//!  * **Installing a driver needs administrator rights**, so [`install_driver`]
//!    elevates with a UAC prompt (`Start-Process -Verb RunAs` → `pnputil`). The
//!    app's installer also stages the driver at setup time (see
//!    `docs/windows-driver.md`); this runtime path is the in-app *install / repair*
//!    action for when that didn't happen or was declined.
//!  * **The driver binary cannot be built or signed off-Windows.** The signed
//!    package (`.inf` + `.sys` + `.cat`) is produced on Windows per
//!    `docs/windows-driver.md` and shipped as an app resource. Until it is present
//!    these functions degrade gracefully: [`routing_device_available`] returns
//!    `false` and [`install_driver`] returns a clear, actionable error.

#![cfg(target_os = "windows")]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::AudioError;

/// Whether the virtual routing device the pipeline needs is present and active —
/// i.e. the bundled driver is installed and working. This is the user-facing
/// "is system-wide EQ ready to use" signal on Windows.
pub fn routing_device_available() -> bool {
    crate::system_eq_windows::available()
}

/// The single `.inf` in a bundled driver package directory, if present.
///
/// We discover it rather than hard-code a name so the build/sign pipeline can drop
/// the upstream package **as-is** — renaming a signed `.inf` would break its `.cat`
/// (the catalog hashes the original filename). Device detection keys off the
/// *friendly name* (`HypeMuzik`), not the filename, so the file name is free.
pub fn find_driver_inf(package_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(package_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("inf"))
        })
}

/// Install (stage + install) the bundled virtual-audio driver from `package_dir`
/// (the folder holding the signed `.inf`/`.sys`/`.cat`).
///
/// Elevates with a UAC prompt and runs `pnputil /add-driver <inf> /install`.
/// Returns `Ok(())` once the elevated `pnputil` process completes; because the
/// elevation boundary hides `pnputil`'s exit code, the caller should re-query
/// [`routing_device_available`] afterwards to confirm the device enumerated
/// (Plug-and-Play can take a moment to surface a freshly installed device).
pub fn install_driver(package_dir: &Path) -> Result<(), AudioError> {
    let inf_path = find_driver_inf(package_dir).ok_or_else(|| {
        AudioError::Unavailable(format!(
            "no driver .inf found in {} — build and sign the package per \
             docs/windows-driver.md, then bundle it as an app resource",
            package_dir.display()
        ))
    })?;
    let inf = inf_path
        .to_str()
        .ok_or_else(|| AudioError::Unavailable("driver .inf path is not valid UTF-8".into()))?;
    // `pnputil` requires admin; elevate via PowerShell `Start-Process` (raises the
    // UAC prompt). `-Wait` blocks until the elevated process exits. Its exit code
    // does not cross the elevation boundary back to us, so a non-error return here
    // means "the installer ran" — success is confirmed by re-checking the device.
    // The path is passed as its own argument (quoted) so spaces are safe.
    let inf_arg = inf.replace('\'', "");
    let ps = format!(
        "Start-Process pnputil -Verb RunAs -Wait -ArgumentList '/add-driver','{inf_arg}','/install'"
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status()
        .map_err(|e| AudioError::Stream(format!("could not launch the driver installer: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AudioError::Stream(
            "driver installation was cancelled or failed — administrator rights are required"
                .into(),
        ))
    }
}
