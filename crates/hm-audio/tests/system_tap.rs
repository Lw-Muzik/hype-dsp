//! Runtime smoke test for the macOS Core Audio process-tap FFI.
//!
//! This does NOT verify that system-wide EQ audibly works (that needs the
//! audio-capture permission granted on a signed build). It verifies the FFI
//! plumbing — PID translation, tap creation, aggregate device, IO proc — runs
//! without crashing/UB and returns a clean `Ok`/`Err`.
//!
//! ```sh
//! cargo test -p hm-audio --test system_tap -- --ignored --nocapture
//! ```

#![cfg(target_os = "macos")]

use std::sync::Arc;

use hm_audio::system_tap::{CaptureTelemetry, SystemTapSource};
use hm_core::SystemEqScope;

#[test]
#[ignore = "creates a Core Audio tap; may trigger the audio-capture permission prompt"]
fn tap_creation_does_not_crash() {
    // Default scope = whole system (the pre-per-app-selection behaviour).
    match SystemTapSource::new(
        48_000,
        Arc::new(CaptureTelemetry::default()),
        &SystemEqScope::default(),
    ) {
        Ok(source) => {
            eprintln!("system tap created OK");
            drop(source);
        }
        Err(e) => {
            // Denied/unavailable is a fine outcome here — the point is that the
            // FFI executed cleanly and surfaced a typed error, not a crash.
            eprintln!("system tap creation returned: {e}");
        }
    }
}
