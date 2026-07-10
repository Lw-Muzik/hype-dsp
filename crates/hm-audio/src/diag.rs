//! Temporary diagnostic logging for debugging the system-wide tap path.
//!
//! Appends to `/tmp/hypemuzik-diag.log` so we can capture ground truth from a
//! bundled GUI app (where stderr is invisible). Remove once the tap is fixed.

use std::io::Write;

/// Append one line to the diagnostic log (best-effort; ignores errors).
///
/// This opens + writes a file, so it must NEVER be called from a real-time audio
/// callback (the capture io_proc, the output callback). Non-RT threads only — the
/// engine control thread and the tap watchdog.
pub(crate) fn log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/hypemuzik-diag.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}
