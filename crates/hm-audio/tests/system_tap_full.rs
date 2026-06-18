//! Headless end-to-end exercise of the system-tap path: tap -> ring -> chain
//! -> output. Captures render-stage diagnostics to /tmp/hypemuzik-diag.log.
//!
//! Run with audio playing through the machine so the tap has real stereo to
//! capture (the harness plays a sound via `afplay`):
//!
//! ```sh
//! cargo test -p hm-audio --test system_tap_full -- --ignored --nocapture
//! ```

#![cfg(target_os = "macos")]

use hm_audio::AudioEngine;
use std::time::Duration;

#[test]
#[ignore = "creates a Core Audio tap + output stream; needs the capture permission"]
fn system_tap_full_path() {
    let engine = AudioEngine::new();

    // Dramatic EQ so any chain effect is unmistakable: full-tilt low boost,
    // deep high cut. If render logs show post != pre, the chain is live.
    let mut bands = [0.0_f32; hm_core::BAND_COUNT];
    for (i, b) in bands.iter_mut().enumerate() {
        *b = if i < hm_core::BAND_COUNT / 2 { 12.0 } else { -24.0 };
    }
    engine.set_power(true);
    engine.set_eq(bands, 0.0, true);

    match engine.play_system_tap() {
        Ok(()) => eprintln!("play_system_tap OK"),
        Err(e) => {
            eprintln!("play_system_tap failed: {e}");
            return;
        }
    }

    // Let the IO proc + render loop run while audio plays.
    std::thread::sleep(Duration::from_millis(2500));
    drop(engine);
    std::thread::sleep(Duration::from_millis(200));
}
