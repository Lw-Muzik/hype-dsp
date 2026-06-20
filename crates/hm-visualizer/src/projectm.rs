//! Safe-ish wrapper over the `projectm-sys` FFI for the MilkDrop renderer.
//!
//! projectM v4 splits the instance (preset evaluation + OpenGL rendering) from
//! the playlist (preset cycling). This wraps just the calls the renderer needs:
//! create/destroy, resize, feed PCM, render a frame, tune behaviour, and a
//! playlist to auto-cycle the bundled `.milk` presets. All GL work happens
//! inside projectM, so the caller only needs a current OpenGL context.

#![cfg(feature = "milkdrop")]
#![allow(dead_code)] // used by the `visualizer` binary; the lib alone doesn't.

use std::ffi::CString;
use std::path::Path;

use projectm_sys as ffi;

/// A projectM instance bound to the current OpenGL context.
pub struct ProjectM {
    handle: ffi::projectm_handle,
}

impl ProjectM {
    /// Create an instance. A current GL context must already exist.
    pub fn new() -> Self {
        let handle = unsafe { ffi::projectm_create() };
        assert!(!handle.is_null(), "projectm_create returned null");
        Self { handle }
    }

    pub fn set_window_size(&self, width: u32, height: u32) {
        unsafe { ffi::projectm_set_window_size(self.handle, width as usize, height as usize) };
    }

    /// Feed a mono PCM window — projectM does its own FFT + beat detection.
    pub fn add_pcm_mono(&self, samples: &[f32]) {
        let max = unsafe { ffi::projectm_pcm_get_max_samples() } as usize;
        let n = samples.len().min(max);
        if n == 0 {
            return;
        }
        unsafe {
            ffi::projectm_pcm_add_float(
                self.handle,
                samples.as_ptr(),
                n as u32,
                ffi::projectm_channels_PROJECTM_MONO,
            );
        }
    }

    /// Render one frame into the current GL framebuffer.
    pub fn render_frame(&self) {
        unsafe { ffi::projectm_opengl_render_frame(self.handle) };
    }

    pub fn set_fps(&self, fps: i32) {
        unsafe { ffi::projectm_set_fps(self.handle, fps) };
    }
    pub fn set_beat_sensitivity(&self, sensitivity: f32) {
        unsafe { ffi::projectm_set_beat_sensitivity(self.handle, sensitivity) };
    }
    pub fn set_preset_duration(&self, seconds: f64) {
        unsafe { ffi::projectm_set_preset_duration(self.handle, seconds) };
    }
    pub fn set_preset_locked(&self, locked: bool) {
        unsafe { ffi::projectm_set_preset_locked(self.handle, locked) };
    }

    /// Load one preset file directly (app-driven selection), blending from the
    /// current preset when `smooth`. Bypasses the playlist's cycling.
    pub fn load_preset_file(&self, path: &Path, smooth: bool) {
        if let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) {
            unsafe { ffi::projectm_load_preset_file(self.handle, c.as_ptr(), smooth) };
        }
    }

    fn handle(&self) -> ffi::projectm_handle {
        self.handle
    }
}

impl Default for ProjectM {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ProjectM {
    fn drop(&mut self) {
        unsafe { ffi::projectm_destroy(self.handle) };
    }
}

/// A preset playlist that cycles the bundled `.milk` files.
pub struct Playlist {
    handle: *mut ffi::projectm_playlist,
}

impl Playlist {
    pub fn new(pm: &ProjectM) -> Self {
        let handle = unsafe { ffi::projectm_playlist_create(pm.handle()) };
        assert!(!handle.is_null(), "projectm_playlist_create returned null");
        Self { handle }
    }

    /// Add every preset under `dir` (recursively). Returns the playlist size.
    pub fn add_dir(&self, dir: &Path) -> u32 {
        if let Ok(c) = CString::new(dir.to_string_lossy().as_bytes()) {
            unsafe { ffi::projectm_playlist_add_path(self.handle, c.as_ptr(), true, false) };
        }
        unsafe { ffi::projectm_playlist_size(self.handle) }
    }

    pub fn set_shuffle(&self, shuffle: bool) {
        unsafe { ffi::projectm_playlist_set_shuffle(self.handle, shuffle) };
    }
    pub fn next(&self) {
        unsafe { ffi::projectm_playlist_play_next(self.handle, true) };
    }
    pub fn prev(&self) {
        unsafe { ffi::projectm_playlist_play_previous(self.handle, true) };
    }
}

impl Drop for Playlist {
    fn drop(&mut self) {
        unsafe { ffi::projectm_playlist_destroy(self.handle) };
    }
}
