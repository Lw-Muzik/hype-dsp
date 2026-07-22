//! One-click Windows system-EQ routing setup — the interim, driverless path.
//!
//! The zero-setup destination for Windows system-wide EQ is the bundled, signed
//! HypeMuzik virtual audio driver (`docs/windows-driver.md`), which needs an EV
//! certificate + Microsoft attestation signing that don't exist yet. Until they
//! do, this module delivers the same "click once and it works" experience with
//! VB-Audio's freely downloadable VB-CABLE:
//!
//!   download (pinned URL) → SHA-256 verify → extract → silent install
//!   (one UAC prompt) → wait for the device to enumerate → ready.
//!
//! The pipeline detects the cable via `hm_audio::system_eq_windows`'s routing
//! candidates ("HypeMuzik" first, then "CABLE Input"), so once the branded
//! driver ships this path demotes to a fallback automatically — nothing to undo.
//!
//! **Safety:** the downloaded installer runs elevated, so it is never executed
//! unless its SHA-256 matches [`VBCABLE_SHA256`], pinned from the official
//! `download.vb-audio.com` package. When VB-Audio publishes a new pack the
//! download fails **closed** (hash mismatch) and the user is pointed at the
//! manual install; bump [`VBCABLE_URL`]/[`VBCABLE_SHA256`] together to update.
//!
//! **Licensing:** VB-CABLE is VB-Audio donationware. The download comes from
//! their official servers on explicit user action only — distribution of this
//! flow in a commercial build should be covered by an agreement with VB-Audio
//! (see `docs/windows-driver.md`).

use hm_core::IpcError;

/// Official VB-CABLE driver-pack download (VB-Audio's server; version-pinned).
// Only *constructed*/read by the Windows setup path (+ tests) — dead elsewhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub const VBCABLE_URL: &str =
    "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";

/// SHA-256 of the exact pack at [`VBCABLE_URL`] (computed 2026-07-22). The
/// installer is never run unless the download matches this.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub const VBCABLE_SHA256: &str =
    "b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb";

/// The silent-install executable inside the pack (x64).
#[cfg(target_os = "windows")]
const VBCABLE_SETUP_EXE: &str = "VBCABLE_Setup_x64.exe";

/// How the one-click setup ended, when it didn't error.
// The type is every platform's command return type; the variants are only ever
// constructed by the Windows arm.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum RoutingSetupOutcome {
    /// The routing device is enumerated and system-wide EQ can be enabled.
    Ready,
    /// The installer ran but the device hasn't enumerated — Windows wants a
    /// reboot (VB-Audio's documented expectation) before it appears.
    NeedsReboot,
}

/// Lowercase-hex SHA-256 of `bytes`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Set up the Windows system-EQ routing device end-to-end: download VB-CABLE
/// from VB-Audio's official server, verify it, and silently install it (one UAC
/// prompt). Emits `system-eq-setup-phase` events (`"downloading"` /
/// `"installing"` / `"detecting"`) so the UI can narrate progress. Errors are
/// user-facing and actionable (manual-install fallback included).
// `(async)`: network download + elevated installer — never main-thread work.
#[tauri::command(async)]
pub fn system_audio_setup_routing(app: tauri::AppHandle) -> Result<RoutingSetupOutcome, IpcError> {
    #[cfg(target_os = "windows")]
    {
        setup_routing_windows(&app).map_err(|e| IpcError::new("cable", e))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err(IpcError::new(
            "cable",
            "system-EQ routing setup is only needed on Windows",
        ))
    }
}

#[cfg(target_os = "windows")]
fn setup_routing_windows(app: &tauri::AppHandle) -> Result<RoutingSetupOutcome, String> {
    use tauri::{Emitter, Manager};

    // Progress narration is best-effort; setup must not fail over UI events.
    let emit = |phase: &str| {
        let _ = app.emit("system-eq-setup-phase", phase);
    };

    // Already there (bundled driver, or a cable installed manually)? Done.
    if hm_audio::win_driver::routing_device_available() {
        return Ok(RoutingSetupOutcome::Ready);
    }

    emit("downloading");
    let bytes = download(VBCABLE_URL)?;
    if sha256_hex(&bytes) != VBCABLE_SHA256 {
        return Err(format!(
            "the downloaded installer failed verification (VB-Audio may have \
             published a new version) — install VB-CABLE manually from \
             https://vb-audio.com/Cable/ and try Enable again"
        ));
    }

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("could not resolve the app cache folder: {e}"))?
        .join("vbcable");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create the download folder: {e}"))?;
    let zip_path = dir.join("VBCABLE_Driver_Pack.zip");
    std::fs::write(&zip_path, &bytes)
        .map_err(|e| format!("could not save the installer: {e}"))?;

    emit("installing");
    expand_archive(&zip_path, &dir)?;
    let setup = dir.join(VBCABLE_SETUP_EXE);
    if !setup.exists() {
        return Err(format!(
            "the installer archive did not contain {VBCABLE_SETUP_EXE} — install \
             VB-CABLE manually from https://vb-audio.com/Cable/"
        ));
    }
    // `-i -h`: install, hidden window (VB-Audio's documented silent switches).
    run_elevated(&setup, &["-i", "-h"])?;

    // The device usually enumerates right away; VB-Audio officially recommends
    // a reboot, so absence after the grace period is "reboot", not failure.
    emit("detecting");
    for _ in 0..20 {
        if hm_audio::win_driver::routing_device_available() {
            return Ok(RoutingSetupOutcome::Ready);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(RoutingSetupOutcome::NeedsReboot)
}

/// Fetch `url` fully into memory (the pack is ~1.3 MB).
#[cfg(target_os = "windows")]
fn download(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("could not set up the download: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("download failed: {e}"))
}

/// A path as a PowerShell single-quoted-literal argument. Single quotes are
/// stripped (they can't legally appear in these app-controlled paths) so the
/// quoting can't be broken out of.
#[cfg(target_os = "windows")]
fn path_arg(p: &std::path::Path) -> Result<String, String> {
    let s = p
        .to_str()
        .ok_or_else(|| "path is not valid UTF-8".to_string())?;
    Ok(s.replace('\'', ""))
}

/// Run a PowerShell command with no visible console window.
#[cfg(target_os = "windows")]
fn powershell(command: &str) -> Result<std::process::ExitStatus, String> {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW` — this is a GUI app; child consoles must not flash.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("could not run PowerShell: {e}"))
}

/// Extract a zip with the built-in `Expand-Archive` (no unzip dependency).
#[cfg(target_os = "windows")]
fn expand_archive(zip: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let zip_arg = path_arg(zip)?;
    let dest_arg = path_arg(dest)?;
    let status = powershell(&format!(
        "Expand-Archive -LiteralPath '{zip_arg}' -DestinationPath '{dest_arg}' -Force"
    ))?;
    if status.success() {
        Ok(())
    } else {
        Err("could not extract the installer archive".into())
    }
}

/// Run `exe` elevated (UAC prompt) and wait for it. As with `pnputil` in
/// `win_driver`, the exit code doesn't cross the elevation boundary — success
/// is confirmed by the device re-check afterwards.
#[cfg(target_os = "windows")]
fn run_elevated(exe: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let exe_arg = path_arg(exe)?;
    let arg_list = args
        .iter()
        .map(|a| format!("'{a}'"))
        .collect::<Vec<_>>()
        .join(",");
    let status = powershell(&format!(
        "Start-Process -FilePath '{exe_arg}' -Verb RunAs -Wait -ArgumentList {arg_list}"
    ))?;
    if status.success() {
        Ok(())
    } else {
        Err("installation was cancelled or failed — administrator approval is required".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FIPS 180-2 / well-known reference vectors.
    #[test]
    fn sha256_hex_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pinned_hash_is_lowercase_hex_sha256_shaped() {
        // Guards against a mangled paste when the pack version is bumped.
        assert_eq!(VBCABLE_SHA256.len(), 64);
        assert!(VBCABLE_SHA256
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(VBCABLE_URL.starts_with("https://download.vb-audio.com/"));
    }
}
