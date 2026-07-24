//! `hm-apo` — the HypeMuzik system-wide EQ Audio Processing Object DLL.
//!
//! A COM in-proc server the Windows audio engine loads into `audiodg.exe`. This
//! module provides the DLL entry points (`DllGetClassObject`, `DllCanUnloadNow`,
//! `DllRegisterServer`, `DllUnregisterServer`) and the class factory that makes
//! the [`apo::HypeMuzikApo`] object; the object itself does the processing.
//!
//! Only the Windows target compiles anything here — on other hosts the crate is
//! empty so the workspace still builds. It cannot be exercised off Windows;
//! runtime validation happens in `audiodg.exe` on a real machine.

#![cfg(windows)]

mod apo;
mod guids;

use core::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, ERROR_SUCCESS, E_UNEXPECTED, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_core::{implement, Interface, BOOL, GUID, HRESULT, PCWSTR};

use crate::guids::CLSID_HYPEMUZIK_APO;

/// This DLL's module handle, captured in `DllMain`, used to resolve our own path
/// for `InprocServer32`.
static DLL_MODULE: AtomicIsize = AtomicIsize::new(0);

/// COM class factory for [`apo::HypeMuzikApo`].
#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<'_, windows_core::IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if !punkouter.is_null() {
            // No aggregation.
            return Err(windows_core::Error::from_hresult(E_UNEXPECTED));
        }
        let unknown: windows_core::IUnknown = apo::HypeMuzikApo::new().into();
        unsafe { unknown.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

/// COM entry point: hand back a class factory for our CLSID.
///
/// # Safety
/// Called by COM with valid `rclsid`/`riid`/`ppv` pointers.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || *rclsid != CLSID_HYPEMUZIK_APO {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    factory.query(riid, ppv)
}

/// COM entry point. We keep the DLL resident (S_FALSE) rather than track a module
/// ref count — an audio effect stays loaded for the session, and never unloading
/// while `audiodg` may hold objects is the safe, simple choice.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

/// Register the CLSID → this DLL, plus the AudioProcessingObjects catalog entry.
/// The installer additionally attaches us to an endpoint and sets
/// `DisableProtectedAudioDG`; this is the COM half so `regsvr32` also works.
///
/// # Safety
/// Standard COM self-registration entry point.
#[no_mangle]
pub unsafe extern "system" fn DllRegisterServer() -> HRESULT {
    match register() {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

/// # Safety
/// Standard COM self-unregistration entry point.
#[no_mangle]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    let _ = delete_key(hm_core::apo_ids::CLSID_REGKEY);
    let _ = delete_key(hm_core::apo_ids::APO_REGKEY);
    S_OK
}

/// Standard DLL entry point; captures the module handle for `InprocServer32`.
///
/// # Safety
/// Called by the loader.
#[no_mangle]
pub unsafe extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DLL_MODULE.store(module.0 as isize, Ordering::Relaxed);
    }
    BOOL(1)
}

fn register() -> windows_core::Result<()> {
    let dll_path = module_path();
    // CLSID → InprocServer32.
    let inproc = format!("{}\\InprocServer32", hm_core::apo_ids::CLSID_REGKEY);
    set_default_str(&inproc, &dll_path)?;
    set_named_str(&inproc, "ThreadingModel", "Both")?;
    // AudioProcessingObjects catalog entry (name is enough for the engine to list
    // us; the endpoint attach is done by the installer's FxProperties writes).
    set_named_str(
        hm_core::apo_ids::APO_REGKEY,
        "FriendlyName",
        "HypeMuzik System Effect",
    )?;
    Ok(())
}

/// The absolute path of this DLL (from the captured module handle).
fn module_path() -> String {
    let module = HMODULE(DLL_MODULE.load(Ordering::Relaxed) as *mut c_void);
    let mut buf = [0u16; 260];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Create `HKLM\<subkey>` and set its default (unnamed) value to `value`.
fn set_default_str(subkey: &str, value: &str) -> windows_core::Result<()> {
    set_str(subkey, None, value)
}

/// Create `HKLM\<subkey>` and set a named string value.
fn set_named_str(subkey: &str, name: &str, value: &str) -> windows_core::Result<()> {
    set_str(subkey, Some(name), value)
}

fn set_str(subkey: &str, name: Option<&str>, value: &str) -> windows_core::Result<()> {
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
            return Err(windows_core::Error::from_hresult(HRESULT(rc.0 as i32)));
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
            return Err(windows_core::Error::from_hresult(HRESULT(rc.0 as i32)));
        }
    }
    Ok(())
}

fn delete_key(subkey: &str) -> windows_core::Result<()> {
    let subkey_w = to_wide(subkey);
    unsafe {
        let rc = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(subkey_w.as_ptr()));
        if rc != ERROR_SUCCESS {
            return Err(windows_core::Error::from_hresult(HRESULT(rc.0 as i32)));
        }
    }
    Ok(())
}
