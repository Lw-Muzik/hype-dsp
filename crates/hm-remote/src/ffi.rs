//! C-ABI for the Flutter phone app (`dart:ffi`). Thin wrappers over
//! [`PhoneNode`](crate::phone::PhoneNode).
//!
//! Memory rules:
//! * `hm_phone_start` returns an opaque handle; free it with `hm_phone_stop`.
//! * functions returning `*mut c_char` transfer ownership — free with
//!   `hm_string_free`. A null return means failure.
//! * all `*const c_char` inputs must be valid NUL-terminated UTF-8.

use crate::phone::PhoneNode;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

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
/// Returns an opaque handle, or null on failure.
///
/// # Safety
/// `secret_path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_start(
    secret_path: *const c_char,
    shelf_port: u16,
) -> *mut PhoneNode {
    let Some(path) = borrow_str(secret_path) else {
        return std::ptr::null_mut();
    };
    match PhoneNode::start(PathBuf::from(path), shelf_port) {
        Ok(node) => Box::into_raw(Box::new(node)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// This phone's stable iroh id (caller frees with `hm_string_free`).
///
/// # Safety
/// `node` must be a handle from `hm_phone_start` that hasn't been stopped.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_endpoint_id(node: *mut PhoneNode) -> *mut c_char {
    let Some(node) = node.as_ref() else {
        return std::ptr::null_mut();
    };
    into_c_string(node.endpoint_id())
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
    let (Some(node), Some(ep), Some(pin), Some(name), Some(token)) = (
        node.as_ref(),
        borrow_str(desktop_ep),
        borrow_str(pin),
        borrow_str(name),
        borrow_str(token),
    ) else {
        return std::ptr::null_mut();
    };
    match node.pair(ep, pin, name, token) {
        Ok(desktop_name) => into_c_string(desktop_name),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Stop the phone node and free its handle.
///
/// # Safety
/// `node` must be a handle from `hm_phone_start`, freed at most once.
#[no_mangle]
pub unsafe extern "C" fn hm_phone_stop(node: *mut PhoneNode) {
    if !node.is_null() {
        drop(Box::from_raw(node));
    }
}

/// Free a string returned by this library.
///
/// # Safety
/// `s` must be a pointer returned by this library, freed at most once.
#[no_mangle]
pub unsafe extern "C" fn hm_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
