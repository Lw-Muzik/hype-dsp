//! End-to-end radio streaming test: open a live HTTP stream, decode it, route
//! it through the engine, and confirm the meters register the signal.
//!
//! Marked `#[ignore]` — it requires network and a real output device. Run:
//!
//! ```sh
//! cargo test -p hm-audio --test radio_playback -- --ignored --nocapture
//! ```

use std::thread::sleep;
use std::time::{Duration, Instant};

use hm_audio::{list_output_devices, AudioEngine};

// SomaFM Groove Salad — a long-standing, reliable public 128k MP3 stream.
const STREAM_URL: &str = "https://ice1.somafm.com/groovesalad-128-mp3";

#[test]
#[ignore = "streams a live internet radio station over the network"]
fn streams_radio_and_meters_move() {
    if list_output_devices().map(|d| d.is_empty()).unwrap_or(true) {
        eprintln!("no output device available; skipping radio test");
        return;
    }

    let engine = AudioEngine::new();
    engine
        .play_radio(STREAM_URL.to_string())
        .expect("play_radio should dispatch");

    // Streaming needs time to connect + prebuffer.
    let start = Instant::now();
    let mut saw_signal = false;
    while start.elapsed() < Duration::from_secs(15) {
        sleep(Duration::from_millis(100));
        if engine.is_playing() && engine.meters().load().peak[0] > 1e-4 {
            saw_signal = true;
            break;
        }
    }
    engine.stop();

    assert!(
        saw_signal,
        "expected the radio stream to buffer and play (network required)"
    );
}
