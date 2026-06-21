//! C-ABI for the Flutter phone app (`dart:ffi`). Thin wrappers over
//! [`PhoneNode`](crate::phone::PhoneNode).
//!
//! Memory rules:
//! * `hm_phone_start` returns an opaque handle; free it with `hm_phone_stop`.
//! * functions returning `*mut c_char` transfer ownership — free with
//!   `hm_string_free`. A null return means failure.
//! * all `*const c_char` inputs must be valid NUL-terminated UTF-8.
//!
//! Panic safety: every entry point runs inside [`guard_ffi`]. A Rust panic that
//! unwound across this `extern "C"` boundary would be undefined behaviour and
//! aborts the whole app (SIGABRT). Instead we catch it and return the failure
//! sentinel (null / no-op), so the Dart side degrades gracefully — e.g. the
//! phone keeps sharing over the LAN even when the iroh endpoint can't start
//! (iroh's relay TLS isn't yet wired up for Android).

use crate::phone::PhoneNode;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

/// Emit a diagnostic line. On Android this goes to `logcat` (visible in
/// `flutter run` / `adb logcat`); elsewhere to stderr. Rust's stdout/stderr is
/// NOT wired to logcat by default, so panic/bind-failure messages would
/// otherwise be lost on device — making the iroh-on-Android failure invisible.
pub(crate) fn diag(msg: &str) {
    #[cfg(target_os = "android")]
    {
        use std::os::raw::{c_char as cc, c_int};
        #[link(name = "log")]
        extern "C" {
            // liblog: int __android_log_write(int prio, const char* tag, const char* text)
            fn __android_log_write(prio: c_int, tag: *const cc, text: *const cc) -> c_int;
        }
        if let (Ok(tag), Ok(text)) = (CString::new("hm-remote"), CString::new(msg)) {
            // 6 = ANDROID_LOG_ERROR
            unsafe { __android_log_write(6, tag.as_ptr(), text.as_ptr()) };
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        eprintln!("{msg}");
    }
}

/// Run an FFI body, converting any panic into `fallback` instead of letting it
/// unwind across the C boundary (UB → process abort). The closure is asserted
/// unwind-safe because on the panic path we discard all of its state and hand
/// the caller the failure sentinel.
fn guard_ffi<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            diag(&format!("caught panic at FFI boundary: {msg}"));
            fallback
        }
    }
}

/// # Safety
/// `p` must be null or a valid NUL-terminated C string outliving the call.
unsafe fn borrow_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn into_c_string(s: String) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Start the phone node: load/persist the identity at `secret_path`, bind the
/// iroh endpoint, and serve the media tunnel into `127.0.0.1:shelf_port`.
/// Returns an opaque handle, or null on failure (including if the node panics
/// while binding — see [`guard_ffi`]).
///
/// # Safety
/// `secret_path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_start(
    secret_path: *const c_char,
    shelf_port: u16,
) -> *mut PhoneNode {
    guard_ffi(std::ptr::null_mut(), || {
        let Some(path) = (unsafe { borrow_str(secret_path) }) else {
            return std::ptr::null_mut();
        };
        match PhoneNode::start(PathBuf::from(path), shelf_port) {
            Ok(node) => Box::into_raw(Box::new(node)),
            Err(e) => {
                diag(&format!("endpoint failed to start: {e:#}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// This phone's stable iroh id (caller frees with `hm_string_free`).
///
/// # Safety
/// `node` must be a handle from `hm_phone_start` that hasn't been stopped.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_endpoint_id(node: *mut PhoneNode) -> *mut c_char {
    guard_ffi(std::ptr::null_mut(), || {
        let Some(node) = (unsafe { node.as_ref() }) else {
            return std::ptr::null_mut();
        };
        into_c_string(node.endpoint_id())
    })
}

/// Pair with the desktop scanned from its QR. Returns the desktop's name on
/// success, or null on failure. Blocking — call off the UI isolate.
///
/// # Safety
/// `node` must be a live handle; the string args valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_pair(
    node: *mut PhoneNode,
    desktop_ep: *const c_char,
    pin: *const c_char,
    name: *const c_char,
    token: *const c_char,
) -> *mut c_char {
    guard_ffi(std::ptr::null_mut(), || {
        let (Some(node), Some(ep), Some(pin), Some(name), Some(token)) = (
            unsafe { node.as_ref() },
            unsafe { borrow_str(desktop_ep) },
            unsafe { borrow_str(pin) },
            unsafe { borrow_str(name) },
            unsafe { borrow_str(token) },
        ) else {
            return std::ptr::null_mut();
        };
        match node.pair(ep, pin, name, token) {
            Ok(desktop_name) => into_c_string(desktop_name),
            Err(e) => {
                diag(&format!("pair (dial) failed: {e:#}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// Stop the phone node and free its handle.
///
/// # Safety
/// `node` must be a handle from `hm_phone_start`, freed at most once.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_stop(node: *mut PhoneNode) {
    guard_ffi((), || {
        if !node.is_null() {
            drop(unsafe { Box::from_raw(node) });
        }
    })
}

/// Free a string returned by this library.
///
/// # Safety
/// `s` must be a pointer returned by this library, freed at most once.
#[no_mangle]
pub unsafe extern "C" fn hm_string_free(s: *mut c_char) {
    guard_ffi((), || {
        if !s.is_null() {
            drop(unsafe { CString::from_raw(s) });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::guard_ffi;

    #[test]
    fn guard_ffi_returns_fallback_on_panic() {
        // A panic inside the body must NOT unwind out of guard_ffi (which, across
        // the real extern "C" boundary, would abort the process). It returns the
        // failure sentinel instead.
        let result = guard_ffi(std::ptr::null_mut::<u8>(), || panic!("boom"));
        assert!(result.is_null());
    }

    #[test]
    fn guard_ffi_passes_value_through_when_ok() {
        assert_eq!(guard_ffi(-1, || 42), 42);
    }
}
