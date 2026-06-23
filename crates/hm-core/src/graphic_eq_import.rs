//! Import EqualizerAPO `GraphicEQ` curves (the AutoEQ interchange format) onto
//! the 31-band graphic EQ, with a clip-proof recommended preamp. Pure data
//! transformation — no DSP, no I/O.

use crate::types::{BAND_COUNT, ISO_CENTERS_HZ};

/// Parse a `GraphicEQ` string into sorted (frequency Hz, gain dB) points.
/// Accepts an optional `GraphicEQ:` label and `freq gain` pairs separated by
/// `;`. Whitespace-tolerant.
pub fn parse_graphic_eq(input: &str) -> Result<Vec<(f32, f32)>, String> {
    let body = input
        .trim()
        .strip_prefix("GraphicEQ:")
        .or_else(|| input.trim().strip_prefix("GraphicEQ"))
        .unwrap_or(input)
        .trim_start_matches([':', ' ']);
    let mut points = Vec::new();
    for pair in body.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.split_whitespace();
        let f = it
            .next()
            .ok_or_else(|| format!("missing frequency in '{pair}'"))?
            .parse::<f32>()
            .map_err(|e| format!("bad frequency '{pair}': {e}"))?;
        let g = it
            .next()
            .ok_or_else(|| format!("missing gain in '{pair}'"))?
            .parse::<f32>()
            .map_err(|e| format!("bad gain '{pair}': {e}"))?;
        points.push((f, g));
    }
    if points.is_empty() {
        return Err("no (freq, gain) points found".into());
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(points)
}

/// Interpolate a (freq, gain) curve onto the ISO band centers in the log-freq
/// domain. Endpoints clamp to the nearest available point.
pub fn interpolate_to_iso_bands(curve: &[(f32, f32)]) -> [f32; BAND_COUNT] {
    let mut out = [0.0f32; BAND_COUNT];
    if curve.is_empty() {
        return out;
    }
    for (i, &center) in ISO_CENTERS_HZ.iter().enumerate() {
        let lc = center.max(1.0).log10();
        if center <= curve[0].0 {
            out[i] = curve[0].1;
            continue;
        }
        if center >= curve[curve.len() - 1].0 {
            out[i] = curve[curve.len() - 1].1;
            continue;
        }
        // Find the bracketing pair.
        let mut j = 0;
        while j + 1 < curve.len() && curve[j + 1].0 < center {
            j += 1;
        }
        let (f0, g0) = curve[j];
        let (f1, g1) = curve[j + 1];
        let l0 = f0.max(1.0).log10();
        let l1 = f1.max(1.0).log10();
        let t = if (l1 - l0).abs() < f32::EPSILON {
            0.0
        } else {
            (lc - l0) / (l1 - l0)
        };
        out[i] = g0 + (g1 - g0) * t;
    }
    out
}

/// Clip-proof preamp: enough negative gain that the peak band reaches 0 dB.
pub fn recommended_preamp(bands: &[f32; BAND_COUNT]) -> f32 {
    let peak = bands.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    -peak.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labeled_and_unlabeled() {
        let a = parse_graphic_eq("GraphicEQ: 20 -1.0; 1000 0.0; 20000 -3.0").unwrap();
        let b = parse_graphic_eq("20 -1.0; 1000 0.0; 20000 -3.0").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert_eq!(a[1], (1000.0, 0.0));
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_graphic_eq("").is_err());
        assert!(parse_graphic_eq("GraphicEQ: ").is_err());
        assert!(parse_graphic_eq("100 ; 200 1").is_err());
    }

    #[test]
    fn interpolation_hits_exact_points() {
        // A curve with a point exactly at the 1 kHz ISO center reproduces it.
        let idx = ISO_CENTERS_HZ.iter().position(|&f| (f - 1000.0).abs() < 0.5).unwrap();
        let curve = vec![(20.0, 0.0), (1000.0, 6.0), (20000.0, 0.0)];
        let bands = interpolate_to_iso_bands(&curve);
        assert!((bands[idx] - 6.0).abs() < 1e-3, "got {}", bands[idx]);
    }

    #[test]
    fn preamp_is_clip_proof() {
        let curve = vec![(20.0, 3.0), (1000.0, 9.0), (20000.0, -2.0)];
        let bands = interpolate_to_iso_bands(&curve);
        let pre = recommended_preamp(&bands);
        let peak = bands.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(peak + pre <= 1e-4, "peak {peak} + preamp {pre} must be <= 0");
    }

    #[test]
    fn preamp_zero_when_no_positive_gain() {
        // An all-attenuation curve needs no headroom → preamp is exactly 0.
        let bands = [-3.0f32; BAND_COUNT];
        assert_eq!(recommended_preamp(&bands), 0.0);
    }

    #[test]
    fn single_point_curve_clamps_all_bands() {
        // A one-point curve applies that gain to every ISO band.
        let bands = interpolate_to_iso_bands(&[(1000.0, 4.0)]);
        assert!(bands.iter().all(|&g| (g - 4.0).abs() < 1e-4), "all bands should be 4.0 dB, got {bands:?}");
    }
}
