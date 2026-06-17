//! Built-in genre EQ presets.
//!
//! Each preset is defined by a handful of `(frequency, gain_dB)` anchor points
//! and interpolated (in log-frequency) onto the 31 ISO band centers, so the
//! curves read clearly and stay smooth across the bands.

use crate::{EqPreset, BAND_COUNT, ISO_CENTERS_HZ};

/// Interpolate a gain (dB) at `freq` from log-frequency anchor points.
fn interp_log(anchors: &[(f32, f32)], freq: f32) -> f32 {
    if anchors.is_empty() {
        return 0.0;
    }
    let lf = freq.log10();
    if lf <= anchors[0].0.log10() {
        return anchors[0].1;
    }
    for pair in anchors.windows(2) {
        let (f0, g0) = pair[0];
        let (f1, g1) = pair[1];
        if lf <= f1.log10() {
            let t = (lf - f0.log10()) / (f1.log10() - f0.log10());
            return g0 + (g1 - g0) * t;
        }
    }
    anchors[anchors.len() - 1].1
}

fn curve(anchors: &[(f32, f32)]) -> [f32; BAND_COUNT] {
    std::array::from_fn(|i| {
        let g = interp_log(anchors, ISO_CENTERS_HZ[i]);
        // Round to 0.1 dB for clean display.
        (g * 10.0).round() / 10.0
    })
}

fn preset(id: &str, name: &str, anchors: &[(f32, f32)]) -> EqPreset {
    EqPreset {
        id: format!("builtin:{id}"),
        name: name.to_string(),
        builtin: true,
        bands: curve(anchors),
        pre_gain: 0.0,
    }
}

/// The shipped genre presets, in display order.
pub fn builtins() -> Vec<EqPreset> {
    vec![
        preset("flat", "Flat", &[]),
        preset(
            "bass-boost",
            "Bass Boost",
            &[
                (20.0, 7.0),
                (60.0, 6.0),
                (150.0, 4.0),
                (300.0, 1.5),
                (500.0, 0.0),
            ],
        ),
        preset(
            "bass-reducer",
            "Bass Reducer",
            &[(20.0, -6.0), (80.0, -4.0), (200.0, -2.0), (400.0, 0.0)],
        ),
        preset(
            "treble-boost",
            "Treble Boost",
            &[(2000.0, 0.0), (4000.0, 2.0), (8000.0, 5.0), (16000.0, 6.0)],
        ),
        preset(
            "vocal",
            "Vocal",
            &[
                (20.0, -2.0),
                (200.0, -1.0),
                (1000.0, 3.0),
                (3000.0, 4.0),
                (6000.0, 1.0),
                (20000.0, -1.0),
            ],
        ),
        preset(
            "rock",
            "Rock",
            &[
                (20.0, 5.0),
                (60.0, 4.0),
                (200.0, 1.0),
                (800.0, -1.5),
                (3000.0, 1.0),
                (8000.0, 4.0),
                (16000.0, 5.0),
            ],
        ),
        preset(
            "pop",
            "Pop",
            &[
                (20.0, -1.0),
                (100.0, 0.0),
                (400.0, 2.0),
                (1500.0, 3.0),
                (4000.0, 1.5),
                (10000.0, -0.5),
            ],
        ),
        preset(
            "electronic",
            "Electronic",
            &[
                (20.0, 6.0),
                (60.0, 5.0),
                (250.0, 1.0),
                (1000.0, 0.0),
                (3000.0, 1.0),
                (8000.0, 3.0),
                (16000.0, 5.0),
            ],
        ),
        preset(
            "jazz",
            "Jazz",
            &[
                (20.0, 3.0),
                (100.0, 2.0),
                (500.0, 0.0),
                (2000.0, 1.0),
                (6000.0, 2.0),
                (16000.0, 3.0),
            ],
        ),
        preset(
            "classical",
            "Classical",
            &[
                (20.0, 4.0),
                (80.0, 3.0),
                (400.0, 0.0),
                (4000.0, 0.0),
                (8000.0, 2.0),
                (16000.0, 3.0),
            ],
        ),
        preset(
            "loudness",
            "Loudness",
            &[
                (20.0, 7.0),
                (60.0, 6.0),
                (200.0, 2.0),
                (1000.0, -1.0),
                (3000.0, 1.0),
                (8000.0, 5.0),
                (16000.0, 7.0),
            ],
        ),
        preset(
            "podcast",
            "Podcast",
            &[
                (20.0, -6.0),
                (100.0, -2.0),
                (300.0, 1.0),
                (2000.0, 3.0),
                (5000.0, 2.0),
                (10000.0, -2.0),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_preset_is_all_zero() {
        let flat = &builtins()[0];
        assert_eq!(flat.id, "builtin:flat");
        assert!(flat.bands.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn bass_boost_lifts_lows_not_highs() {
        let bass = builtins()
            .into_iter()
            .find(|p| p.id == "builtin:bass-boost")
            .unwrap();
        // Band 0 = 20 Hz, band 30 = 20 kHz.
        assert!(bass.bands[0] > 4.0, "expected strong low boost");
        assert!(bass.bands[30].abs() < 0.5, "highs should be untouched");
    }
}
