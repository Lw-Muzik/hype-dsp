//! Import ViPER4Android / JamesDSP **DDC** (`.vdc`) files onto the 31-band
//! graphic EQ. Pure data transformation — no DSP state, no I/O.
//!
//! A `.vdc` stores a cascade of biquad sections, pre-computed per sample rate,
//! one line per rate:
//!
//! ```text
//! SR_44100:b0,b1,b2,a1,a2,b0,b1,b2,a1,a2,...
//! SR_48000:...
//! ```
//!
//! Each section is five doubles `b0,b1,b2,a1,a2`. The reference DDCToolbox
//! importer stores the feedback terms negated (`a1 = -file_a1`, `a2 = -file_a2`),
//! giving the Direct-Form-II transfer function
//! `H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²)` once negated. We
//! evaluate the cascade's magnitude response at the ISO band centers and map it
//! onto the graphic EQ — exactly like the GraphicEQ import, with the same
//! clip-proof [`recommended_preamp`](crate::recommended_preamp). Sampling the
//! response only at the 31 ISO centers is an approximation (a narrow peak
//! between centers is missed) — the same trade-off the graphic EQ already makes.

use crate::types::{BAND_COUNT, ISO_CENTERS_HZ};

/// A parsed `.vdc`: the sample rate its coefficients target and the biquad
/// cascade (each `[b0, b1, b2, a1, a2]`).
#[derive(Debug, Clone, PartialEq)]
pub struct VdcCurve {
    pub sample_rate: f64,
    pub biquads: Vec<[f64; 5]>,
}

/// Sample-rate preference when a `.vdc` carries several `SR_<rate>` lines: the
/// Hz response is the same across them, so 44.1 kHz is chosen first, then 48 kHz,
/// then whatever appears first. Lower rank wins.
fn rate_rank(rate: f64) -> u8 {
    if (rate - 44_100.0).abs() < 1.0 {
        0
    } else if (rate - 48_000.0).abs() < 1.0 {
        1
    } else {
        2
    }
}

/// Parse a `.vdc` file body. Picks one `SR_<rate>:` line (see [`rate_rank`]) and
/// reads its comma-separated coefficients into biquad sections of five.
///
/// Errors if there is no `SR_<rate>` line, the line has no coefficients, a
/// coefficient is unparseable, or the count is not a multiple of five.
pub fn parse_vdc(input: &str) -> Result<VdcCurve, String> {
    let mut chosen: Option<(f64, &str)> = None;
    for line in input.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("SR_") else {
            continue;
        };
        let Some((rate_str, body)) = rest.split_once(':') else {
            continue;
        };
        let Ok(rate) = rate_str.trim().parse::<f64>() else {
            continue;
        };
        let take = match chosen {
            None => true,
            Some((cur, _)) => rate_rank(rate) < rate_rank(cur),
        };
        if take {
            chosen = Some((rate, body));
        }
    }
    let (sample_rate, body) = chosen.ok_or("no SR_<rate> line found in .vdc")?;
    if !(sample_rate.is_finite() && sample_rate > 0.0) {
        return Err(format!("invalid sample rate {sample_rate}"));
    }

    let mut coeffs = Vec::new();
    for tok in body.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let v = tok
            .parse::<f64>()
            .map_err(|e| format!("bad coefficient '{tok}': {e}"))?;
        if !v.is_finite() {
            return Err(format!("non-finite coefficient '{tok}'"));
        }
        coeffs.push(v);
    }
    if coeffs.is_empty() {
        return Err("no coefficients found in .vdc".into());
    }
    if coeffs.len() % 5 != 0 {
        return Err(format!(
            "coefficient count {} is not a multiple of 5 (b0,b1,b2,a1,a2 per section)",
            coeffs.len()
        ));
    }
    // The reference DDCToolbox importer stores a1/a2 NEGATED (`a1 = -val`), so the
    // denominator `1 + a1·z⁻¹ + a2·z⁻²` uses the negated 4th/5th file values.
    let biquads = coeffs
        .chunks_exact(5)
        .map(|c| [c[0], c[1], c[2], -c[3], -c[4]])
        .collect();
    Ok(VdcCurve {
        sample_rate,
        biquads,
    })
}

/// Magnitude (dB) of the biquad cascade at `freq_hz`, evaluated for `sample_rate`.
/// Mirrors the reference DDCToolbox magnitude-response routine.
fn cascade_response_db(biquads: &[[f64; 5]], sample_rate: f64, freq_hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq_hz / sample_rate;
    // z⁻¹ = e^{-jw}, z⁻² = e^{-j2w}
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let (z1_re, z1_im) = (c1, -s1);
    let (z2_re, z2_im) = (c2, -s2);

    let mut h_re = 1.0;
    let mut h_im = 0.0;
    for &[b0, b1, b2, a1, a2] in biquads {
        let num_re = b0 + b1 * z1_re + b2 * z2_re;
        let num_im = b1 * z1_im + b2 * z2_im;
        let den_re = 1.0 + a1 * z1_re + a2 * z2_re;
        let den_im = a1 * z1_im + a2 * z2_im;
        let den_mag2 = den_re * den_re + den_im * den_im;
        if den_mag2 < f64::EPSILON {
            // Pole on the unit circle — the reference treats this as a null.
            return f64::NEG_INFINITY;
        }
        // h *= num; then h /= den (single complex divide).
        let (nr, ni) = (h_re * num_re - h_im * num_im, h_re * num_im + h_im * num_re);
        h_re = (nr * den_re + ni * den_im) / den_mag2;
        h_im = (ni * den_re - nr * den_im) / den_mag2;
    }
    let mag = (h_re * h_re + h_im * h_im).sqrt();
    20.0 * mag.max(1e-12).log10()
}

/// Parse a `.vdc` and resolve its magnitude response onto the 31 ISO band gains
/// (dB). Feed the result to [`recommended_preamp`](crate::recommended_preamp) +
/// the engine EQ, exactly like a GraphicEQ import.
pub fn vdc_to_iso_bands(input: &str) -> Result<[f32; BAND_COUNT], String> {
    let curve = parse_vdc(input)?;
    let mut bands = [0.0f32; BAND_COUNT];
    for (i, &center) in ISO_CENTERS_HZ.iter().enumerate() {
        let db = cascade_response_db(&curve.biquads, curve.sample_rate, center as f64);
        // A null (pole on the unit circle) maps to deep attenuation, not -inf.
        bands[i] = if db.is_finite() { db as f32 } else { -120.0 };
    }
    Ok(bands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_biquad_is_flat_zero_db() {
        // H(z) = 1 → 0 dB at every band.
        let c = parse_vdc("SR_48000:1,0,0,0,0").unwrap();
        assert_eq!(c.sample_rate, 48_000.0);
        assert_eq!(c.biquads.len(), 1);
        let bands = vdc_to_iso_bands("SR_48000:1,0,0,0,0").unwrap();
        assert!(bands.iter().all(|&g| g.abs() < 1e-4), "got {bands:?}");
    }

    #[test]
    fn pure_gain_biquad_is_flat_gain() {
        // b0 = 2 → +6.02 dB at every band.
        let bands = vdc_to_iso_bands("SR_44100:2,0,0,0,0").unwrap();
        let expect = 20.0 * 2.0_f32.log10();
        assert!(
            bands.iter().all(|&g| (g - expect).abs() < 1e-3),
            "expected {expect} dB, got {bands:?}"
        );
    }

    #[test]
    fn prefers_44100_then_48000() {
        let body = "SR_48000:2,0,0,0,0\nSR_44100:4,0,0,0,0\nSR_96000:8,0,0,0,0";
        let c = parse_vdc(body).unwrap();
        assert_eq!(c.sample_rate, 44_100.0);
        assert_eq!(c.biquads[0][0], 4.0);
    }

    #[test]
    fn falls_back_to_first_when_no_preferred_rate() {
        let c = parse_vdc("SR_96000:1,0,0,0,0\nSR_88200:1,0,0,0,0").unwrap();
        assert_eq!(c.sample_rate, 96_000.0);
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_vdc("").is_err()); // no SR line
        assert!(parse_vdc("SR_48000:").is_err()); // no coefficients
        assert!(parse_vdc("SR_48000:1,0,0,0").is_err()); // not a multiple of 5
        assert!(parse_vdc("SR_48000:1,0,oops,0,0").is_err()); // bad number
    }

    #[test]
    fn parses_scientific_notation_and_multiple_sections() {
        // Two sections; the second uses E-notation as real .vdc files do.
        let c = parse_vdc("SR_44100:1,0,0,0,0,1.0,-5.2E-14,0,0,0").unwrap();
        assert_eq!(c.biquads.len(), 2);
        assert!((c.biquads[1][1] - -5.2e-14).abs() < 1e-20);
    }

    #[test]
    fn real_world_section_yields_finite_bounded_response() {
        // A real first section from a Beyerdynamic DT770 .vdc (oratory-style
        // peaking filter): the response must be finite and modest at every band.
        let vdc = "SR_44100:0.998734355950096,-1.99282366314795,0.994268042195585,1.99282366314795,-0.993002398145681";
        let bands = vdc_to_iso_bands(vdc).unwrap();
        assert!(bands.iter().all(|g| g.is_finite()), "got {bands:?}");
        assert!(
            bands.iter().all(|&g| g.abs() < 24.0),
            "a single gentle peaking section shouldn't exceed ±24 dB: {bands:?}"
        );
    }
}
