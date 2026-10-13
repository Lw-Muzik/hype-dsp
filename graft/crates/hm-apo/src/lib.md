# crates/hm-apo/src/lib.rs

- apo · module · L14-L14 — mod apo;
- guids · module · L15-L15 — mod guids;
- DLL_MODULE · constant · L36-L36 — static DLL_MODULE: AtomicIsize = AtomicIsize::new(0);
- Factory · struct · L40-L40 — struct Factory;
- CreateInstance · function · L43-L55 — fn CreateInstance(
- LockServer · function · L57-L59 — fn LockServer(&self, _flock: BOOL) -> windows_core::Result<()>
- DllGetClassObject · function · L67-L77 — pub unsafe extern "system" fn DllGetClassObject(
- DllCanUnloadNow · function · L83-L85 — pub extern "system" fn DllCanUnloadNow() -> HRESULT
- DllRegisterServer · function · L94-L99 — pub unsafe extern "system" fn DllRegisterServer() -> HRESULT
- DllUnregisterServer · function · L104-L108 — pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT
- DllMain · function · L115-L120 — pub unsafe extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL
- register · function · L122-L136 — fn register() -> windows_core::Result<()>
- module_path · function · L139-L144 — fn module_path() -> String
- to_wide · function · L146-L148 — fn to_wide(s: &str) -> Vec<u16>
- set_default_str · function · L151-L153 — fn set_default_str(subkey: &str, value: &str) -> windows_core::Result<()>
- set_named_str · function · L156-L158 — fn set_named_str(subkey: &str, name: &str, value: &str) -> windows_core::Result<()>
- set_str · function · L160-L192 — fn set_str(subkey: &str, name: Option<&str>, value: &str) -> windows_core::Result<()>
- delete_key · function · L194-L203 — fn delete_key(subkey: &str) -> windows_core::Result<()>
