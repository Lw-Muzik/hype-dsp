# crates/hm-remote/src/ffi.rs

- diag · function · L27-L45 — pub(crate) fn diag(msg: &str)
- __android_log_write · function · L34-L34 — fn __android_log_write(prio: c_int, tag: *const cc, text: *const cc) -> c_int;
- guard_ffi · function · L51-L64 — fn guard_ffi<T>(fallback: T, f: impl FnOnce() -> T) -> T
- borrow_str · function · L68-L73 — unsafe fn borrow_str<'a>(p: *const c_char) -> Option<&'a str>
- into_c_string · function · L75-L79 — fn into_c_string(s: String) -> *mut c_char
- hm_phone_start · function · L89-L105 — pub unsafe extern "C" fn hm_phone_start(
- hm_phone_endpoint_id · function · L112-L119 — pub unsafe extern "C" fn hm_phone_endpoint_id(node: *mut PhoneNode) -> *mut c_char
- hm_phone_pair · function · L127-L152 — pub unsafe extern "C" fn hm_phone_pair(
- hm_phone_stop · function · L159-L165 — pub unsafe extern "C" fn hm_phone_stop(node: *mut PhoneNode)
- hm_string_free · function · L172-L178 — pub unsafe extern "C" fn hm_string_free(s: *mut c_char)
- tests · module · L181-L197 — mod tests
- guard_ffi_returns_fallback_on_panic · function · L185-L191 — fn guard_ffi_returns_fallback_on_panic()
- guard_ffi_passes_value_through_when_ok · function · L194-L196 — fn guard_ffi_passes_value_through_when_ok()
