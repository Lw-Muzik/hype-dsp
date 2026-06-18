//! Temporary diagnostic logging for debugging the system-wide tap path.
//!
//! Appends to `/tmp/hypemuzik-diag.log` so we can capture ground truth from a
//! bundled GUI app (where stderr is invisible). Remove once the tap is fixed.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

/// Append one line to the diagnostic log (best-effort; ignores errors).
pub(crate) fn log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/hypemuzik-diag.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// True for the first `n` increments of `counter` — rate-limits hot-path logs.
pub(crate) fn first_n(counter: &AtomicU32, n: u32) -> bool {
    counter.fetch_add(1, Ordering::Relaxed) < n
}
