//! Windows system-wide EQ via our own **Audio Processing Object** (the free,
//! no-cert, no-account path). This is the app-side half; the APO DLL that runs
//! inside `audiodg.exe` is the `hm-apo` crate.
//!
//! The pure planning logic — backend selection and which endpoint `FxProperties`
//! slots to write — is always compiled and unit-tested on the dev host. The
//! Windows-only IO (streaming live params to the APO, probing whether the APO is
//! installed) is gated to the Windows target. The *elevated installer* that
//! writes the `FxProperties` values, sets `DisableProtectedAudioDG`, and
//! copies/registers the DLL lives in `commands/apo_setup.rs` (it needs elevation)
//! and uses the pure helpers here.

use hm_core::apo_ids;

/// Which Windows system-EQ backend to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum WindowsBackend {
    /// A bundled signed virtual audio device is present (the premium path).
    SignedDriver,
    /// Our APO is installed (the free path).
    Apo,
    /// Neither — offer setup.
    None,
}

/// Pick the backend. Priority: signed driver > our APO > none — so a signed
/// driver shipped later supersedes the APO with nothing to undo. Pure so it is
/// unit-tested on the dev host.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn select(driver_present: bool, apo_is_installed: bool) -> WindowsBackend {
    if driver_present {
        WindowsBackend::SignedDriver
    } else if apo_is_installed {
        WindowsBackend::Apo
    } else {
        WindowsBackend::None
    }
}

/// Which of an endpoint's two APO effect slots we occupy. Composite/Bluetooth
/// endpoints don't honour the EFX (endpoint) slot, so we use MFX (mode) there —
/// matching EasyEffects' `DeviceAPOInfo` logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum ApoSlot {
    /// Stream + Endpoint effect slots (pids 5 and 7) — the normal case.
    SfxEfx,
    /// Stream + Mode effect slots (pids 5 and 6) — composite/Bluetooth endpoints.
    SfxMfx,
}

/// Choose the effect slot for an endpoint. Pure so it is unit-tested on the host.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn choose_slot(is_composite: bool) -> ApoSlot {
    if is_composite {
        ApoSlot::SfxMfx
    } else {
        ApoSlot::SfxEfx
    }
}

/// The two `FxProperties` value names (`"{PKEY},pid"`) to write our CLSID into
/// for the chosen slot. Pure so it is unit-tested on the host.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn fx_value_names(slot: ApoSlot) -> [String; 2] {
    let pk = apo_ids::FX_PROPERTIES_PKEY;
    match slot {
        ApoSlot::SfxEfx => [format!("{pk},5"), format!("{pk},7")],
        ApoSlot::SfxMfx => [format!("{pk},5"), format!("{pk},6")],
    }
}

/// The registry key holding a render endpoint's `FxProperties`. Pure so it is
/// unit-tested on the host.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn endpoint_fx_key(endpoint_guid: &str) -> String {
    format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render\\{endpoint_guid}\\FxProperties"
    )
}

// ---------------------------------------------------------------------------
// Windows-only IO: live param feed + install probe.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use arc_swap::ArcSwap;
    use hm_core::apo_ids;
    use hm_core::apo_ipc::{write_seqlock, EngineParamsPod, SharedMapping};
    use hm_core::EngineState;

    use crate::error::AudioError;

    /// A running APO param feed. The APO stays *installed* across sessions; this
    /// only streams live params and flips the `active` gate. Dropping it tells the
    /// APO to pass audio through (active = 0).
    pub struct ApoBackend {
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl ApoBackend {
        /// Open the shared mapping and start streaming `state` to the APO.
        pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
            let writer = SharedMapping::create_writer(apo_ids::MAPPING_NAME)
                .map_err(|e| AudioError::Stream(format!("APO shared memory: {e}")))?;
            let stop = Arc::new(AtomicBool::new(false));
            let run = stop.clone();
            let worker = std::thread::Builder::new()
                .name("hm-apo-params".into())
                .spawn(move || {
                    // ~60 Hz is ample for parameter changes; the audio itself is
                    // processed at the graph rate inside audiodg.
                    while !run.load(Ordering::Relaxed) {
                        let pod = EngineParamsPod::from_state(&state.load(), true);
                        write_seqlock(writer.cell(), &pod);
                        std::thread::sleep(Duration::from_millis(16));
                    }
                    // Final snapshot with active = 0 so the APO passes audio through.
                    let pod = EngineParamsPod::from_state(&state.load(), false);
                    write_seqlock(writer.cell(), &pod);
                })
                .map_err(|e| AudioError::Stream(format!("APO param worker: {e}")))?;
            Ok(Self {
                stop,
                worker: Some(worker),
            })
        }
    }

    impl Drop for ApoBackend {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
        }
    }

    use std::path::{Path, PathBuf};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        STGM_READWRITE,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    use super::{choose_slot, endpoint_fx_key};

    /// The `FxProperties` PKEY fmtid ({d04e05a6-594b-4fb6-a80d-01af5eed7d1d}); our
    /// APO CLSID goes into pids 5/6/7 of it depending on the slot.
    const FX_FMTID: windows_core::GUID =
        windows_core::GUID::from_u128(0xd04e05a6_594b_4fb6_a80d_01af5eed7d1d);

    /// Outcome of an install/attach attempt.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ApoInstallOutcome {
        /// Installed and attached; a reboot finalizes it for the audio engine.
        NeedsReboot,
    }

    /// RAII COM init for the calling thread (apartment-threaded).
    struct ComInit;
    impl ComInit {
        fn new() -> Self {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            Self
        }
    }
    impl Drop for ComInit {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    /// Full elevated install: copy the DLL, register the COM class + catalog, set
    /// `DisableProtectedAudioDG`, and attach to the default render endpoint.
    /// **Must run elevated** (HKLM + ProgramFiles). Returns [`ApoInstallOutcome`].
    pub fn install(dll_src: &Path) -> Result<ApoInstallOutcome, AudioError> {
        let dll_dst = copy_dll(dll_src)?;
        register_com(&dll_dst)?;
        set_disable_protected_audio_dg(true)?;
        let _com = ComInit::new();
        attach_default_endpoint()?;
        Ok(ApoInstallOutcome::NeedsReboot)
    }

    /// Reverse [`install`]: detach, unregister, clear the global flag.
    pub fn uninstall() -> Result<(), AudioError> {
        {
            let _com = ComInit::new();
            let _ = detach_default_endpoint();
        }
        let _ = unregister_com();
        let _ = set_disable_protected_audio_dg(false);
        Ok(())
    }

    /// If installed but not attached to the *current* default endpoint (a Windows
    /// update wiped it, or the default changed), re-attach. No-op otherwise.
    pub fn repair() -> Result<(), AudioError> {
        if !apo_installed() {
            return Ok(());
        }
        let _com = ComInit::new();
        if !apo_attached().unwrap_or(false) {
            attach_default_endpoint()?;
        }
        Ok(())
    }

    /// Copy the bundled DLL to `%ProgramFiles%\HypeMuzik\apo\hm_apo.dll`.
    fn copy_dll(src: &Path) -> Result<PathBuf, AudioError> {
        let base = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .ok_or_else(|| AudioError::Stream("ProgramFiles not set".into()))?;
        let dir = base.join("HypeMuzik").join("apo");
        std::fs::create_dir_all(&dir).map_err(|e| AudioError::Stream(format!("apo dir: {e}")))?;
        let dst = dir.join("hm_apo.dll");
        std::fs::copy(src, &dst).map_err(|e| AudioError::Stream(format!("copy apo dll: {e}")))?;
        Ok(dst)
    }

    /// Register the CLSID → DLL (InprocServer32) + the AudioProcessingObjects
    /// catalog entry. (The DLL also self-registers via `DllRegisterServer`; doing
    /// it here keeps the installer self-contained.)
    fn register_com(dll_path: &Path) -> Result<(), AudioError> {
        let path = dll_path.to_string_lossy();
        let inproc = format!("{}\\InprocServer32", hm_core::apo_ids::CLSID_REGKEY);
        reg_set_str(&inproc, None, &path)?;
        reg_set_str(&inproc, Some("ThreadingModel"), "Both")?;
        reg_set_str(
            hm_core::apo_ids::APO_REGKEY,
            Some("FriendlyName"),
            "HypeMuzik System Effect",
        )?;
        Ok(())
    }

    fn unregister_com() -> Result<(), AudioError> {
        reg_delete_tree(hm_core::apo_ids::CLSID_REGKEY)?;
        reg_delete_tree(hm_core::apo_ids::APO_REGKEY)?;
        Ok(())
    }

    /// Set/clear the DWORD that lets audiodg load unsigned APOs.
    fn set_disable_protected_audio_dg(enable: bool) -> Result<(), AudioError> {
        reg_set_dword(
            hm_core::apo_ids::DISABLE_PROTECTED_AUDIO_DG_KEY,
            hm_core::apo_ids::DISABLE_PROTECTED_AUDIO_DG_VALUE,
            u32::from(enable),
        )
    }

    /// Write our CLSID into the default render endpoint's FX slots.
    fn attach_default_endpoint() -> Result<(), AudioError> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| AudioError::Stream(format!("device enumerator: {e}")))?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| AudioError::Stream(format!("default endpoint: {e}")))?;
            let store: IPropertyStore = device
                .OpenPropertyStore(STGM_READWRITE)
                .map_err(|e| AudioError::Stream(format!("open property store: {e}")))?;
            let slot = choose_slot(false); // composite-detection is a follow-up
            for pid in slot_pids(slot) {
                let key = PROPERTYKEY {
                    fmtid: FX_FMTID,
                    pid,
                };
                let pv = PROPVARIANT::from(hm_core::apo_ids::CLSID_STR);
                store
                    .SetValue(&key, &pv)
                    .map_err(|e| AudioError::Stream(format!("set fx value: {e}")))?;
            }
            store
                .Commit()
                .map_err(|e| AudioError::Stream(format!("commit fx: {e}")))?;
        }
        Ok(())
    }

    fn detach_default_endpoint() -> Result<(), AudioError> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| AudioError::Stream(format!("device enumerator: {e}")))?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| AudioError::Stream(format!("default endpoint: {e}")))?;
            let store: IPropertyStore = device
                .OpenPropertyStore(STGM_READWRITE)
                .map_err(|e| AudioError::Stream(format!("open property store: {e}")))?;
            for pid in [5u32, 6, 7] {
                let key = PROPERTYKEY {
                    fmtid: FX_FMTID,
                    pid,
                };
                let _ = store.SetValue(&key, &PROPVARIANT::default());
            }
            let _ = store.Commit();
        }
        Ok(())
    }

    /// Whether the current default endpoint's FX slot already holds our CLSID.
    fn apo_attached() -> Result<bool, AudioError> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| AudioError::Stream(format!("device enumerator: {e}")))?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| AudioError::Stream(format!("default endpoint: {e}")))?;
            let store: IPropertyStore = device
                .OpenPropertyStore(STGM_READWRITE)
                .map_err(|e| AudioError::Stream(format!("open property store: {e}")))?;
            for pid in [5u32, 7, 6] {
                let key = PROPERTYKEY {
                    fmtid: FX_FMTID,
                    pid,
                };
                if let Ok(pv) = store.GetValue(&key) {
                    let s = pv.to_string();
                    if s.to_ascii_uppercase().contains(
                        &hm_core::apo_ids::CLSID_STR.trim_matches(|c| c == '{' || c == '}')
                            .to_ascii_uppercase(),
                    ) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn slot_pids(slot: super::ApoSlot) -> [u32; 2] {
        match slot {
            super::ApoSlot::SfxEfx => [5, 7],
            super::ApoSlot::SfxMfx => [5, 6],
        }
    }

    // --- small registry helpers ------------------------------------------------

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn reg_set_str(subkey: &str, name: Option<&str>, value: &str) -> Result<(), AudioError> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE,
            REG_OPTION_NON_VOLATILE, REG_SZ,
        };
        let subkey_w = to_wide(subkey);
        let mut hkey = HKEY::default();
        unsafe {
            let rc = RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey_w.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            );
            if rc != ERROR_SUCCESS {
                return Err(AudioError::Stream(format!("RegCreateKeyExW {subkey}: {rc:?}")));
            }
            let data = to_wide(value);
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            let name_w = name.map(to_wide);
            let name_ptr = name_w
                .as_ref()
                .map(|n| PCWSTR(n.as_ptr()))
                .unwrap_or(PCWSTR::null());
            let rc = RegSetValueExW(hkey, name_ptr, Some(0), REG_SZ, Some(bytes));
            let _ = RegCloseKey(hkey);
            if rc != ERROR_SUCCESS {
                return Err(AudioError::Stream(format!("RegSetValueExW {subkey}: {rc:?}")));
            }
        }
        Ok(())
    }

    fn reg_set_dword(subkey: &str, name: &str, value: u32) -> Result<(), AudioError> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE,
            REG_DWORD, REG_OPTION_NON_VOLATILE,
        };
        let subkey_w = to_wide(subkey);
        let name_w = to_wide(name);
        let mut hkey = HKEY::default();
        unsafe {
            let rc = RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey_w.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            );
            if rc != ERROR_SUCCESS {
                return Err(AudioError::Stream(format!("RegCreateKeyExW {subkey}: {rc:?}")));
            }
            let bytes = value.to_ne_bytes();
            let rc = RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), Some(0), REG_DWORD, Some(&bytes));
            let _ = RegCloseKey(hkey);
            if rc != ERROR_SUCCESS {
                return Err(AudioError::Stream(format!("RegSetValueExW {subkey}: {rc:?}")));
            }
        }
        Ok(())
    }

    fn reg_delete_tree(subkey: &str) -> Result<(), AudioError> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_LOCAL_MACHINE};
        let subkey_w = to_wide(subkey);
        unsafe {
            let rc = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(subkey_w.as_ptr()));
            if rc != ERROR_SUCCESS {
                return Err(AudioError::Stream(format!("RegDeleteTreeW {subkey}: {rc:?}")));
            }
        }
        Ok(())
    }

    // Referenced by endpoint_fx_key doctest-style callers; keep it used.
    #[allow(dead_code)]
    fn _fx_key_ref(guid: &str) -> String {
        endpoint_fx_key(guid)
    }

    /// Whether our APO's CLSID is registered (i.e. the DLL has been installed).
    pub fn apo_installed() -> bool {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        };
        let subkey: Vec<u16> = apo_ids::CLSID_REGKEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut hkey = HKEY::default();
        unsafe {
            let rc = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_READ,
                &mut hkey,
            );
            if rc == ERROR_SUCCESS {
                let _ = RegCloseKey(hkey);
                true
            } else {
                false
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use win::{apo_installed, install, repair, uninstall, ApoBackend, ApoInstallOutcome};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_priority_driver_then_apo_then_none() {
        assert_eq!(select(true, true), WindowsBackend::SignedDriver);
        assert_eq!(select(true, false), WindowsBackend::SignedDriver);
        assert_eq!(select(false, true), WindowsBackend::Apo);
        assert_eq!(select(false, false), WindowsBackend::None);
    }

    #[test]
    fn composite_endpoints_use_mode_slot() {
        assert_eq!(choose_slot(true), ApoSlot::SfxMfx);
        assert_eq!(choose_slot(false), ApoSlot::SfxEfx);
    }

    #[test]
    fn fx_value_names_match_slot_pids() {
        let pk = apo_ids::FX_PROPERTIES_PKEY;
        assert_eq!(
            fx_value_names(ApoSlot::SfxEfx),
            [format!("{pk},5"), format!("{pk},7")]
        );
        assert_eq!(
            fx_value_names(ApoSlot::SfxMfx),
            [format!("{pk},5"), format!("{pk},6")]
        );
    }

    #[test]
    fn endpoint_fx_key_is_render_scoped() {
        let k = endpoint_fx_key("{abc}");
        assert!(k.contains("MMDevices\\Audio\\Render\\{abc}"));
        assert!(k.ends_with("\\FxProperties"));
    }
}
