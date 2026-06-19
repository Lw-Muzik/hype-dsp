//! HypeMuzik MilkDrop visualizer — a standalone sidecar window.
//!
//! The main Tauri app spawns this binary and pipes it the engine's mono PCM
//! (raw little-endian `f32`, 512-sample frames) over **stdin**. It opens an
//! OpenGL window with SDL2 and drives libprojectM, auto-cycling the bundled
//! `.milk` presets. Running it as its own process gives the window its own
//! main-thread event loop (required on macOS) and isolates the native renderer
//! from the app — a crash here can't take the app down.
//!
//! Usage: `hm-visualizer <preset_dir> [fps] [beat_sensitivity] [preset_secs]`

use std::io::Read;
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

fn main() {
    let preset_dir = std::env::args().nth(1).unwrap_or_default();
    let fps: i32 = arg(2, 30);
    let beat: f32 = arg(3, 1.0);
    let preset_secs: f64 = arg(4, 20.0);

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
    // Keep the context alive for the program's lifetime (drop = teardown).
    let _gl_ctx = window.gl_create_context().expect("GL context");
    window.gl_make_current(&_gl_ctx).expect("make current");
    let _ = video.gl_set_swap_interval(SwapInterval::VSync);

    // --- projectM + preset playlist --------------------------------------------
    let projectm = ProjectM::new();
    projectm.set_window_size(width, height);
    projectm.set_fps(fps);
    projectm.set_beat_sensitivity(beat);
    projectm.set_preset_duration(preset_secs);

    let playlist = Playlist::new(&projectm);
    if !preset_dir.is_empty() {
        let count = playlist.add_dir(std::path::Path::new(&preset_dir));
        if count > 0 {
            playlist.set_shuffle(true);
            playlist.next();
        }
    }

    // --- stdin PCM pump (background thread → latest-frame slot) -----------------
    let pcm: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let pcm = pcm.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut bytes = [0u8; PCM_FRAME * 4];
            while stdin.read_exact(&mut bytes).is_ok() {
                let frame: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                *pcm.lock().expect("pcm poisoned") = frame;
            }
            // stdin closed → the app exited / closed the visualizer.
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
