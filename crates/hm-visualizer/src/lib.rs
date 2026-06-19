//! Native MilkDrop (libprojectM) visualizer.
//!
//! The `milkdrop` feature pulls in `projectm-sys` (which compiles libprojectM v4
//! from source via CMake) and `sdl2`, and builds the `hm-visualizer` sidecar
//! binary (see `main.rs`). It's optional and off by default, so normal workspace
//! builds don't require the native toolchain (CMake / OpenGL / GLEW / SDL2 /
//! vcpkg) — only the dedicated `milkdrop` CI job and the app's release build
//! enable it.
//!
//! [`projectm`] is the safe wrapper over the projectM C API that the sidecar
//! binary drives.

#[cfg(feature = "milkdrop")]
pub mod projectm;

/// Whether this build includes the native libprojectM renderer.
pub const HAS_MILKDROP: bool = cfg!(feature = "milkdrop");
