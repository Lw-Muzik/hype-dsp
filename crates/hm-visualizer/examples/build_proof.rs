//! Cross-platform build proof for the native libprojectM dependency.
//!
//! Built only with `--features milkdrop` (see the `milkdrop` CI job). Compiling
//! and linking this binary confirms CMake built libprojectM and it links on the
//! current OS — the de-risk before the full renderer is implemented.

fn main() {
    println!("hm-visualizer milkdrop = {}", hm_visualizer::HAS_MILKDROP);
    println!("libprojectM linked: {}", hm_visualizer::projectm_linked());
}
