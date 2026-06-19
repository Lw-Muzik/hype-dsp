//! Audio fingerprinting (Chromaprint) for track identification.
//!
//! Decodes a file and produces an AcoustID-compatible fingerprint — the same
//! algorithm and compressed format `fpcalc` emits — plus the track duration, so
//! the caller can look the recording up via the AcoustID web service. Pure Rust
//! (no external `fpcalc` binary) via `rusty-chromaprint`.

use std::path::Path;

use base64::Engine as _;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};

use crate::decode::decode_file;

/// Only the first couple of minutes are needed to identify a recording.
const MAX_FINGERPRINT_SECS: usize = 120;

/// Fingerprint `path` for AcoustID lookup. Returns the base64url (no-pad)
/// compressed fingerprint and the track's duration in seconds, or `None` if the
/// file can't be decoded.
pub fn fingerprint_file(path: &Path) -> Option<(String, u32)> {
    let decoded = decode_file(path).ok()?;
    let rate = decoded.sample_rate;
    if rate == 0 || decoded.samples.is_empty() {
        return None;
    }

    // `decode_file` yields interleaved stereo f32; the full length gives the
    // duration, the leading slice feeds the fingerprint.
    let total_frames = decoded.samples.len() / 2;
    let duration = (total_frames as u64 / rate as u64) as u32;

    let max = MAX_FINGERPRINT_SECS * rate as usize * 2;
    let slice = &decoded.samples[..decoded.samples.len().min(max)];
    let pcm: Vec<i16> = slice
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();

    let config = Configuration::preset_test2(); // AcoustID's default algorithm
    let mut printer = Fingerprinter::new(&config);
    printer.start(rate, 2).ok()?;
    printer.consume(&pcm);
    printer.finish();

    let raw = printer.fingerprint();
    if raw.is_empty() {
        return None;
    }
    let compressed = FingerprintCompressor::from(&config).compress(raw);
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&compressed);
    Some((encoded, duration))
}
