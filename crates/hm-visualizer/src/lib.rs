//! Native MilkDrop (libprojectM) visualizer.
//!
//! The `milkdrop` feature pulls in `projectm-sys`, which compiles libprojectM
//! v4 from source via CMake and links it **statically**. It's optional and off
//! by default, so normal workspace builds don't require the native toolchain
//! (CMake / OpenGL / GLEW / vcpkg) — only the dedicated `milkdrop` CI job enables
//! it to validate the cross-platform native build before the renderer is wired.
//!
//! The renderer itself (a dedicated native GL window driving libprojectM, fed by
//! a lock-free PCM tap off the engine) lands in a follow-up once the build is
//! green on all three platforms.

#[cfg(feature = "milkdrop")]
pub mod projectm;

/// Whether this build includes the native libprojectM renderer.
pub const HAS_MILKDROP: bool = cfg!(feature = "milkdrop");

/// Build/link proof: touches the `projectm-sys` dependency so libprojectM is
/// actually compiled and linked. Returns the opaque instance handle's type name.
#[cfg(feature = "milkdrop")]
pub fn projectm_linked() -> &'static str {
    std::any::type_name::<*mut projectm_sys::projectm>()
}
