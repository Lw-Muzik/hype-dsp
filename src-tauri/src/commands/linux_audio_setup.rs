//! One-click / launch-time system-audio dependency setup for **Linux**.
//!
//! Most Linux users need nothing: on PipeWire (the ~90–95 % case) the system-EQ
//! backend's only runtime need — `libpipewire` and a running daemon — is present
//! by definition, and the `.deb`/`.rpm` already declare `Depends: libpipewire`,
//! so the OS package manager installs it at install time. The one genuinely
//! fixable runtime gap is a **classic-PulseAudio box missing `pulseaudio-utils`**
//! (`parec`/`pacat`), which the Pulse fallback needs.
//!
//! Unlike the Windows VB-CABLE flow, Linux can't install a system package
//! silently: it needs **root** (one `pkexec` polkit prompt) and is
//! **distro-specific**. So this module auto-detects the package manager
//! (apt/dnf/pacman/zypper) and installs the missing package behind a single auth
//! prompt, falling back to a copy-paste command when it can't (Flatpak/Snap
//! sandboxes, unknown distro, or no polkit).
//!
//! [`auto_setup_on_launch`] runs this at startup when — and only when — system EQ
//! is currently unavailable but a package install would fix it; it never nags
//! when nothing is needed, and remembers a declined prompt so it doesn't re-ask
//! every launch (the Settings action can always retry).
//!
//! The pure planning helpers (package-manager argv, package names, missing-set
//! computation) are always compiled and unit-tested on the dev host; only the IO
//! (probing the system, running `pkexec`) is Linux-gated.

use hm_core::IpcError;

/// A host package manager we know how to drive non-interactively.
// The whole planning API below is exercised by the Linux setup path and by the
// unit tests; on a non-Linux non-test build it's all unreferenced.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    /// Debian/Ubuntu/Mint (`apt-get`).
    Apt,
    /// Fedora/RHEL (`dnf`).
    Dnf,
    /// Arch/Manjaro (`pacman`).
    Pacman,
    /// openSUSE (`zypper`).
    Zypper,
}

impl PackageManager {
    /// The manager's executable name (also what we probe on `PATH`).
    /// Only read by the Linux setup path (+ its detection); dead elsewhere.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn binary(self) -> &'static str {
        match self {
            PackageManager::Apt => "apt-get",
            PackageManager::Dnf => "dnf",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
        }
    }

    /// The package that provides `pactl`/`parec`/`pacat` on this distro.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn pulse_utils_package(self) -> &'static str {
        match self {
            // Arch ships the PulseAudio CLI tools in `libpulse`, not a
            // `-utils` package.
            PackageManager::Pacman => "libpulse",
            _ => "pulseaudio-utils",
        }
    }
}

/// The full argv (after `pkexec`) to non-interactively install `pkgs`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn install_argv(pm: PackageManager, pkgs: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = match pm {
        PackageManager::Apt => vec!["apt-get".into(), "install".into(), "-y".into()],
        PackageManager::Dnf => vec!["dnf".into(), "install".into(), "-y".into()],
        PackageManager::Pacman => {
            vec!["pacman".into(), "-S".into(), "--noconfirm".into(), "--needed".into()]
        }
        PackageManager::Zypper => {
            vec!["zypper".into(), "--non-interactive".into(), "install".into()]
        }
    };
    argv.extend(pkgs.iter().map(|p| p.to_string()));
    argv
}

/// The copy-paste command a user can run themselves when we can't auto-install
/// (sandbox / unknown distro / no polkit).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn manual_command(pm: PackageManager, pkgs: &[&str]) -> String {
    format!("sudo {}", install_argv(pm, pkgs).join(" "))
}

/// Compute which packages to install to make a system-EQ backend usable, given a
/// snapshot of the audio stack. Pure so it is unit-tested on the dev host.
///
/// - PipeWire running → nothing (the native backend is ready).
/// - a PulseAudio server reachable but `parec`/`pacat` absent → the Pulse-utils
///   package (enables the fallback backend).
/// - otherwise → nothing installable helps (no server to talk to; we don't
///   install/switch a whole audio server behind the user's back).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn compute_missing_packages(
    pm: PackageManager,
    pipewire_running: bool,
    pulse_reachable: bool,
    parec_present: bool,
    pacat_present: bool,
) -> Vec<&'static str> {
    if pipewire_running {
        return Vec::new();
    }
    if pulse_reachable && (!parec_present || !pacat_present) {
        return vec![pm.pulse_utils_package()];
    }
    Vec::new()
}

/// How a setup attempt resolved. Serialized to the frontend (camelCase tag).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinuxSetupResult {
    /// System EQ is already available; nothing to install.
    AlreadyReady,
    /// The missing package(s) were installed; system EQ is now available.
    Installed,
    /// We can't auto-install here (sandbox / unknown distro / no polkit) — the
    /// user should run `command` themselves.
    NeedsManual { command: String },
    /// Nothing we can install would help (e.g. no audio server running at all),
    /// or this isn't Linux.
    NotApplicable,
    /// The install ran but failed; `command` is the manual fallback.
    Failed { message: String, command: String },
}

/// Manually-triggered setup (from the Settings action). Ignores the
/// declined-marker so an explicit retry always runs. On non-Linux this is a
/// no-op `NotApplicable` (the command is registered on every platform, like the
/// Windows routing command).
#[tauri::command]
pub fn linux_system_audio_setup(app: tauri::AppHandle) -> Result<LinuxSetupResult, IpcError> {
    #[cfg(target_os = "linux")]
    {
        Ok(imp::run_setup(&app, imp::Trigger::Manual))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Ok(LinuxSetupResult::NotApplicable)
    }
}

#[cfg(target_os = "linux")]
pub use imp::auto_setup_on_launch;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use tauri::{Emitter, Manager};

    /// What kicked off a setup run — controls whether the declined-marker is
    /// honoured (auto) or ignored (an explicit manual retry).
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Trigger {
        Auto,
        Manual,
    }

    /// Launch-time entry point: run setup only if system EQ is unavailable, a
    /// package install would fix it, and the user hasn't already declined. Runs
    /// on a background thread so it never blocks startup.
    pub fn auto_setup_on_launch(app: tauri::AppHandle) {
        std::thread::Builder::new()
            .name("hm-linux-audio-setup".into())
            .spawn(move || {
                // Already working (PipeWire, or Pulse with tools present) → nothing.
                if hm_audio::system_eq_available() {
                    return;
                }
                if declined_marker(&app).map(|p| p.exists()).unwrap_or(false) {
                    return;
                }
                let _ = run_setup(&app, Trigger::Auto);
            })
            .ok();
    }

    pub fn run_setup(app: &tauri::AppHandle, trigger: Trigger) -> LinuxSetupResult {
        let emit = |phase: &str| {
            let _ = app.emit("system-eq-setup-phase", phase);
        };
        emit("checking");

        if hm_audio::system_eq_available() {
            return LinuxSetupResult::AlreadyReady;
        }

        let Some(pm) = detect_package_manager() else {
            // Unknown distro: we can still tell the user what to install if we
            // know a plausible package name (default to apt's).
            let cmd = manual_command(PackageManager::Apt, &["pulseaudio-utils"]);
            emit("manual");
            let _ = app.emit("system-eq-setup-manual", &cmd);
            return LinuxSetupResult::NeedsManual { command: cmd };
        };

        let missing = compute_missing_packages(
            pm,
            hm_audio::system_eq_pipewire::available(),
            pactl_reachable(),
            which("parec"),
            which("pacat"),
        );
        if missing.is_empty() {
            // Unavailable but nothing installable helps (no server running).
            return LinuxSetupResult::NotApplicable;
        }

        let manual = manual_command(pm, &missing);

        // Inside a Flatpak/Snap sandbox we can't touch the host package manager.
        if is_sandboxed() {
            emit("manual");
            let _ = app.emit("system-eq-setup-manual", &manual);
            return LinuxSetupResult::NeedsManual { command: manual };
        }

        emit("installing");
        match run_pkexec_install(pm, &missing) {
            Ok(()) => {
                clear_declined_marker(app);
                if hm_audio::system_eq_available() {
                    emit("ready");
                    LinuxSetupResult::Installed
                } else {
                    // Install "succeeded" but EQ still isn't available — surface
                    // honestly rather than claim victory.
                    emit("manual");
                    let _ = app.emit("system-eq-setup-manual", &manual);
                    LinuxSetupResult::NeedsManual { command: manual }
                }
            }
            Err(e) => {
                // On auto runs, remember the decline/failure so we don't re-prompt
                // every launch. A manual retry ignores the marker.
                if trigger == Trigger::Auto {
                    let _ = write_declined_marker(app);
                }
                emit("failed");
                let _ = app.emit("system-eq-setup-manual", &manual);
                LinuxSetupResult::Failed {
                    message: e,
                    command: manual,
                }
            }
        }
    }

    /// Run the install elevated via `pkexec` (one polkit prompt). Never uses a
    /// shell, and the package names come from our fixed allowlist, so there is no
    /// injection surface.
    fn run_pkexec_install(pm: PackageManager, pkgs: &[&str]) -> Result<(), String> {
        if !which("pkexec") {
            return Err("pkexec (PolicyKit) is not installed".into());
        }
        let argv = install_argv(pm, pkgs);
        let status = Command::new("pkexec")
            .args(&argv)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| format!("failed to launch pkexec: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            // pkexec exits 126 when the user dismisses/denies the auth dialog.
            match status.code() {
                Some(126) => Err("authorization was dismissed".into()),
                Some(127) => Err("authorization could not be obtained".into()),
                Some(c) => Err(format!("install failed (exit {c})")),
                None => Err("install was terminated".into()),
            }
        }
    }

    /// First package manager found on `PATH`, in preference order.
    fn detect_package_manager() -> Option<PackageManager> {
        for pm in [
            PackageManager::Apt,
            PackageManager::Dnf,
            PackageManager::Pacman,
            PackageManager::Zypper,
        ] {
            if which(pm.binary()) {
                return Some(pm);
            }
        }
        None
    }

    /// Whether `name` resolves to an executable on `PATH` (no process spawned).
    fn which(name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path).any(|dir| {
            let p = dir.join(name);
            std::fs::metadata(&p).map(|m| m.is_file()).unwrap_or(false)
        })
    }

    /// Whether a PulseAudio-compatible server answers `pactl info`.
    fn pactl_reachable() -> bool {
        Command::new("pactl")
            .arg("info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Running inside a Flatpak/Snap/container sandbox, where we can't drive the
    /// host package manager.
    fn is_sandboxed() -> bool {
        std::env::var_os("FLATPAK_ID").is_some()
            || std::env::var_os("SNAP").is_some()
            || std::env::var_os("container").is_some()
            || std::path::Path::new("/.flatpak-info").exists()
    }

    fn declined_marker(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path()
            .app_config_dir()
            .ok()
            .map(|d| d.join("linux-audio-setup-declined"))
    }

    fn write_declined_marker(app: &tauri::AppHandle) -> std::io::Result<()> {
        if let Some(p) = declined_marker(app) {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(p, b"1")?;
        }
        Ok(())
    }

    fn clear_declined_marker(app: &tauri::AppHandle) {
        if let Some(p) = declined_marker(app) {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_install_argv_is_noninteractive() {
        assert_eq!(
            install_argv(PackageManager::Apt, &["pulseaudio-utils"]),
            vec!["apt-get", "install", "-y", "pulseaudio-utils"]
        );
    }

    #[test]
    fn pacman_uses_libpulse_and_noconfirm() {
        assert_eq!(PackageManager::Pacman.pulse_utils_package(), "libpulse");
        let argv = install_argv(PackageManager::Pacman, &["libpulse"]);
        assert_eq!(argv, vec!["pacman", "-S", "--noconfirm", "--needed", "libpulse"]);
    }

    #[test]
    fn manual_command_is_sudo_prefixed() {
        assert_eq!(
            manual_command(PackageManager::Dnf, &["pulseaudio-utils"]),
            "sudo dnf install -y pulseaudio-utils"
        );
    }

    #[test]
    fn nothing_missing_when_pipewire_is_running() {
        // Even if the Pulse tools are absent, PipeWire being up means the native
        // backend works — install nothing.
        assert!(compute_missing_packages(PackageManager::Apt, true, true, false, false).is_empty());
    }

    #[test]
    fn pulse_utils_missing_installs_the_package() {
        let missing =
            compute_missing_packages(PackageManager::Apt, false, true, false, false);
        assert_eq!(missing, vec!["pulseaudio-utils"]);
        // Only one tool missing still triggers the install.
        let one = compute_missing_packages(PackageManager::Zypper, false, true, true, false);
        assert_eq!(one, vec!["pulseaudio-utils"]);
    }

    #[test]
    fn nothing_installable_when_no_server_is_reachable() {
        assert!(compute_missing_packages(PackageManager::Apt, false, false, false, false).is_empty());
    }

    #[test]
    fn nothing_to_do_when_pulse_tools_present() {
        assert!(compute_missing_packages(PackageManager::Apt, false, true, true, true).is_empty());
    }
}
