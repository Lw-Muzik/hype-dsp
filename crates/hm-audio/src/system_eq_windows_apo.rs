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
pub use win::{apo_installed, ApoBackend};

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
