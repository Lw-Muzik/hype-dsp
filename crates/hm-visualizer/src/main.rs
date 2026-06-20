//! HypeMuzik MilkDrop visualizer — a standalone sidecar window.
//!
//! The main Tauri app spawns this binary and drives it over **stdin** with a
//! tiny tagged protocol so one pipe carries both audio and control:
//!   - `b'P'` + 512 little-endian `f32` (2048 bytes) — a mono PCM frame.
//!   - `b'L'` + `u16` length (LE) + UTF-8 preset name — load that `.milk`.
//!
//! It opens an OpenGL window with SDL2 and drives libprojectM. Running it as its
//! own process gives the window its own main-thread event loop (required on
//! macOS) and isolates the native renderer — a crash here can't take the app
//! down. The app fully drives preset selection (the playlist is kept only for
//! the window's own ←/→ keys), so it never auto-cycles on its own.
//!
//! Usage: `hm-visualizer <preset_dir> [fps] [beat] [preset_secs] [initial_preset]`

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hm_visualizer::projectm::{Playlist, ProjectM};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Keycode;
use sdl2::video::{GLProfile, SwapInterval};

/// PCM frame size piped from the app (matches `WAVEFORM_SAMPLES`).
const PCM_FRAME: usize = 512;

fn arg<T: std::str::FromStr>(n: usize, default: T) -> T {
    std::env::args().nth(n).and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Resolve a preset name to its `.milk` path under `dir`.
fn preset_path(dir: &str, name: &str) -> PathBuf {
    Path::new(dir).join(format!("{name}.milk"))
}

fn main() {
    let preset_dir = std::env::args().nth(1).unwrap_or_default();
    let fps: i32 = arg(2, 30);
    let beat: f32 = arg(3, 1.0);
    let preset_secs: f64 = arg(4, 30.0);
    let initial_preset = std::env::args().nth(5).filter(|s| !s.is_empty());

    // --- Window + OpenGL 3.3 core context --------------------------------------
    let sdl = sdl2::init().expect("SDL init");
    let video = sdl.video().expect("SDL video");
    {
        let gl = video.gl_attr();
        gl.set_context_profile(GLProfile::Core);
        gl.set_context_version(3, 3);
        gl.set_double_buffer(true);
    }
    let (mut width, mut height) = (1280u32, 720u32);
    let window = video
        .window("HypeMuzik Visualizer", width, height)
        .opengl()
        .resizable()
        .position_centered()
        .build()
        .expect("create window");
    let _gl_ctx = window.gl_create_context().expect("GL context");
    window.gl_make_current(&_gl_ctx).expect("make current");
    let _ = video.gl_set_swap_interval(SwapInterval::VSync);

    // --- projectM + preset playlist --------------------------------------------
    let projectm = ProjectM::new();
    projectm.set_window_size(width, height);
    projectm.set_fps(fps);
    projectm.set_beat_sensitivity(beat);
    projectm.set_preset_duration(preset_secs);

    // The playlist is only for the window's own ←/→ keys — the app drives preset
    // changes by name, so projectM should not auto-transition over them.
    projectm.set_preset_locked(true);
    let playlist = Playlist::new(&projectm);
    if !preset_dir.is_empty() {
        playlist.add_dir(Path::new(&preset_dir));
    }
    // Show the app's selected preset on open (else a first one to start with).
    match &initial_preset {
        Some(name) => projectm.load_preset_file(&preset_path(&preset_dir, name), false),
        None => playlist.next(),
    }

    // --- stdin reader (background thread → latest PCM + pending preset) ---------
    let pcm: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let pending: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let pcm = pcm.clone();
        let pending = pending.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut tag = [0u8; 1];
            while stdin.read_exact(&mut tag).is_ok() {
                match tag[0] {
                    b'P' => {
                        let mut bytes = [0u8; PCM_FRAME * 4];
                        if stdin.read_exact(&mut bytes).is_err() {
                            break;
                        }
                        let frame: Vec<f32> = bytes
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        *pcm.lock().expect("pcm poisoned") = frame;
                    }
                    b'L' => {
                        let mut len = [0u8; 2];
                        if stdin.read_exact(&mut len).is_err() {
                            break;
                        }
                        let mut name = vec![0u8; u16::from_le_bytes(len) as usize];
                        if stdin.read_exact(&mut name).is_err() {
                            break;
                        }
                        if let Ok(name) = String::from_utf8(name) {
                            *pending.lock().expect("pending poisoned") = Some(name);
                        }
                    }
                    // Unknown tag = protocol desync; stop reading rather than
                    // misinterpret the rest of the stream.
                    _ => break,
                }
            }
        });
    }

    // --- render loop -----------------------------------------------------------
    let mut events = sdl.event_pump().expect("event pump");
    'main: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => break 'main,
                Event::KeyDown { keycode: Some(Keycode::Right | Keycode::N), .. } => {
                    playlist.next()
                }
                Event::KeyDown { keycode: Some(Keycode::Left | Keycode::P), .. } => {
                    playlist.prev()
                }
                Event::Window { win_event: WindowEvent::Resized(w, h), .. } => {
                    width = w.max(1) as u32;
                    height = h.max(1) as u32;
                    projectm.set_window_size(width, height);
                }
                _ => {}
            }
        }

        // Apply an app-requested preset on the main (GL) thread.
        if let Some(name) = pending.lock().expect("pending poisoned").take() {
            projectm.load_preset_file(&preset_path(&preset_dir, &name), true);
        }

        {
            let frame = pcm.lock().expect("pcm poisoned");
            if !frame.is_empty() {
                projectm.add_pcm_mono(&frame);
            }
        }
        projectm.render_frame();
        window.gl_swap_window();
    }
}
