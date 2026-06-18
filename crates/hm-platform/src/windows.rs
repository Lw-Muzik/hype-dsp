//! Windows per-app mixer via WASAPI audio sessions — the native, no-driver path
//! the OS volume mixer itself uses.
//!
//! Enumerate the default render endpoint's sessions
//! (`IAudioSessionManager2::GetSessionEnumerator`), resolve each session's
//! process to its executable name, and set per-app volume/mute through
//! [`ISimpleAudioVolume`]. Sessions are grouped per executable so one app with
//! several sessions moves together.
//!
//! All COM/Win32 calls are `unsafe` and serialized behind the controller's outer
//! `Mutex`. This file is `cfg(target_os = "windows")` and is built/verified by
//! CI (and on a Windows machine), not on the macOS dev host.

#![cfg(target_os = "windows")]

use std::collections::HashMap;

use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, FALSE, MAX_PATH};
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use hm_core::AppSession;

use crate::error::PlatformError;
use crate::SessionController;

/// One app's audio session group on the default output.
struct SessionInfo {
    /// Stable id: the executable basename, lowercased (e.g. `chrome.exe`).
    id: String,
    name: String,
    volume: f32,
    muted: bool,
}

/// Full executable path for a PID (or `None` for the system / no access).
unsafe fn process_image_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid).ok()?;
    let mut buf = [0u16; MAX_PATH as usize];
    let mut len = buf.len() as u32;
    let res =
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(handle);
    res.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn display_name(exe: &str) -> String {
    let base = basename(exe);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base)
        .to_string()
}

/// Create the default render endpoint's session enumerator. COM is initialized
/// (multithreaded) on the calling thread if needed.
unsafe fn session_enumerator() -> windows::core::Result<IAudioSessionEnumerator> {
    // S_FALSE (already initialized) and RPC_E_CHANGED_MODE are non-fatal.
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
    let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
    manager.GetSessionEnumerator()
}

/// Snapshot all per-app sessions on the default output, grouped by executable.
unsafe fn collect_sessions() -> windows::core::Result<Vec<SessionInfo>> {
    let sessions = session_enumerator()?;
    let count = sessions.GetCount()?;
    let mut map: HashMap<String, SessionInfo> = HashMap::new();
    for i in 0..count {
        let ctrl = match sessions.GetSession(i) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let ctrl2: IAudioSessionControl2 = match ctrl.cast() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let pid = ctrl2.GetProcessId().unwrap_or(0);
        let Some(exe) = process_image_path(pid) else {
            continue; // system sounds / inaccessible
        };
        let id = basename(&exe).to_lowercase();
        if map.contains_key(&id) {
            continue;
        }
        let vol: ISimpleAudioVolume = match ctrl.cast() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let volume = vol.GetMasterVolume().unwrap_or(1.0);
        let muted = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        map.insert(
            id.clone(),
            SessionInfo {
                id,
                name: display_name(&exe),
                volume,
                muted,
            },
        );
    }
    Ok(map.into_values().collect())
}

/// Apply `op` to every `ISimpleAudioVolume` whose executable basename is `id`.
unsafe fn apply(
    id: &str,
    op: impl Fn(&ISimpleAudioVolume) -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    let sessions = session_enumerator()?;
    let count = sessions.GetCount()?;
    for i in 0..count {
        let Ok(ctrl) = sessions.GetSession(i) else {
            continue;
        };
        let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
            continue;
        };
        let pid = ctrl2.GetProcessId().unwrap_or(0);
        let Some(exe) = process_image_path(pid) else {
            continue;
        };
        if basename(&exe).to_lowercase() == id {
            if let Ok(vol) = ctrl.cast::<ISimpleAudioVolume>() {
                let _ = op(&vol);
            }
        }
    }
    Ok(())
}

/// Windows per-app mixer controller (WASAPI session volume).
pub struct WindowsSessionController;

impl WindowsSessionController {
    pub fn new() -> Self {
        Self
    }
}

impl SessionController for WindowsSessionController {
    fn supported(&self) -> bool {
        true
    }

    fn unavailable_reason(&self) -> Option<String> {
        None
    }

    fn list_sessions(&self) -> Vec<AppSession> {
        let mut sessions: Vec<AppSession> = unsafe { collect_sessions() }
            .unwrap_or_default()
            .into_iter()
            .map(|s| AppSession {
                id: s.id,
                name: s.name,
                icon: None,
                volume: s.volume,
                muted: s.muted,
            })
            .collect();
        sessions.sort_by_key(|s| s.name.to_lowercase());
        sessions
    }

    fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError> {
        let level = gain.clamp(0.0, 1.0);
        unsafe {
            apply(id, |vol| vol.SetMasterVolume(level, &GUID::zeroed()))
                .map_err(|e| PlatformError::Unsupported(format!("WASAPI set volume failed: {e}")))
        }
    }

    fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError> {
        let flag = BOOL::from(muted);
        unsafe {
            apply(id, |vol| vol.SetMute(flag, &GUID::zeroed()))
                .map_err(|e| PlatformError::Unsupported(format!("WASAPI set mute failed: {e}")))
        }
    }
}
