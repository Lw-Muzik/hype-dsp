//! End-to-end engine test: decode a real WAV, open the default output device,
//! and confirm playback starts and the meters register the signal.
//!
//! Marked `#[ignore]` because it opens a real audio device and briefly emits
//! sound. Run explicitly:
//!
//! ```sh
//! cargo test -p hm-audio --test engine_playback -- --ignored
//! ```

use std::thread::sleep;
use std::time::{Duration, Instant};

use hm_audio::{list_output_devices, AudioEngine};

fn write_sine_wav(path: &std::path::Path, seconds: f32, sample_rate: u32) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    let frames = (seconds * sample_rate as f32) as usize;
    for f in 0..frames {
        let t = f as f32 / sample_rate as f32;
        let s = (t * 2.0 * std::f32::consts::PI * 440.0).sin() * 0.2;
        let v = (s * i16::MAX as f32) as i16;
        writer.write_sample(v).unwrap();
        writer.write_sample(v).unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
#[ignore = "opens a real audio output device and briefly emits sound"]
fn plays_a_wav_and_meters_move() {
    if list_output_devices().map(|d| d.is_empty()).unwrap_or(true) {
        eprintln!("no output device available; skipping playback test");
        return;
    }

    let path = std::env::temp_dir().join("hm_engine_playback_test.wav");
    write_sine_wav(&path, 2.0, 44_100);

    let engine = AudioEngine::new();
    engine
        .play_file(&path)
        .expect("play_file should decode and start");

    let start = Instant::now();
    let mut saw_signal = false;
    // Generous window: the output device can take a moment to warm up.
    while start.elapsed() < Duration::from_millis(2500) {
        sleep(Duration::from_millis(20));
        if engine.is_playing() && engine.meters().load().peak[0] > 1e-4 {
            saw_signal = true;
            break;
        }
    }
    engine.stop();
    let _ = std::fs::remove_file(&path);

    assert!(
        saw_signal,
        "expected playback to start and the meters to register signal"
    );
}
