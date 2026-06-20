//! Build script.
//!
//! On Windows, SDL2 (statically linked via its `bundled` + `static-link`
//! features) calls Win32 registry APIs — `RegOpenKeyExW` / `RegQueryValueExW` /
//! `RegCloseKey`, in `WIN_LookupAudioDeviceName` — but `sdl2-sys` doesn't list
//! `advapi32` among the system libs it emits. That leaves the static link with
//! unresolved `__imp_Reg*` symbols (LNK2019/LNK1120). Link `advapi32` ourselves.
//!
//! Gated to Windows + the `milkdrop` feature (the only config that pulls in
//! SDL2), so the normal trivial build and macOS/Linux are untouched.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let milkdrop = std::env::var_os("CARGO_FEATURE_MILKDROP").is_some();
    if windows && milkdrop {
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
