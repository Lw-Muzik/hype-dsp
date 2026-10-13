//! Spleeter's soft ratio masks and "average" band extension.
//!
//! All four nets' magnitude estimates are needed: each stem's mask is its
//! squared magnitude over the sum of all four squared magnitudes, so the four
//! masks sum to exactly 1 per bin and the stems always add back to the mix.
//! The nets only see the lowest [`NET_BINS`] bins; above that the mask is the
//! per-frame mean of the net-range mask ("average" extension). Zeros there
//! would leave a −23 dB hole in the top octave.

use crate::config::NET_BINS;

/// Added to the total in the denominator.
pub const TOTAL_EPS: f32 = 1e-10;
/// Added to each numerator (a quarter of [`TOTAL_EPS`], so the four masks still
/// sum to 1 to within rounding even where every net is silent).
pub const NUMERATOR_EPS: f32 = 2.5e-11;

/// Turn the nets' magnitude estimates into the vocals and drums masks, in
/// place: `vocals` and `drums` hold net outputs on entry and masks on return.
/// `bass` and `other` are the remaining two nets' outputs. Same evaluation order
/// as the mobile engine, so the float results agree.
pub fn soft_masks(vocals: &mut [f32], drums: &mut [f32], bass: &[f32], other: &[f32]) {
    debug_assert!(vocals.len() == drums.len() && vocals.len() == bass.len() && vocals.len() == other.len());
    for i in 0..vocals.len() {
        let sum = vocals[i] * vocals[i] + drums[i] * drums[i] + bass[i] * bass[i];
        let total = sum + other[i] * other[i] + TOTAL_EPS;
        vocals[i] = (vocals[i] * vocals[i] + NUMERATOR_EPS) / total;
        drums[i] = (drums[i] * drums[i] + NUMERATOR_EPS) / total;
    }
}

/// The mean of every `NET_BINS`-wide row of `mask` (one value per frame and
/// channel) — the mask value used for the bins above the nets' range.
pub fn row_means(mask: &[f32], out: &mut [f32]) {
    debug_assert_eq!(mask.len(), out.len() * NET_BINS);
    for (row, m) in out.iter_mut().zip(mask.chunks_exact(NET_BINS)) {
        let sum: f64 = m.iter().map(|&x| f64::from(x)).sum();
        *row = (sum / NET_BINS as f64) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_masks_sum_to_one() {
        let v0 = [0.3f32, 0.0, 2.0, 1e-6];
        let d0 = [0.1f32, 0.0, 0.5, 4.0];
        let b0 = [0.8f32, 0.0, 0.1, 0.0];
        let o0 = [0.2f32, 0.0, 0.7, 1e-3];
        let (mut v, mut d) = (v0, d0);
        soft_masks(&mut v, &mut d, &b0, &o0);
        for i in 0..4 {
            // masks of bass/other by the same formula
            let total = v0[i].powi(2) + d0[i].powi(2) + b0[i].powi(2) + o0[i].powi(2) + TOTAL_EPS;
            let mb = (b0[i].powi(2) + NUMERATOR_EPS) / total;
            let mo = (o0[i].powi(2) + NUMERATOR_EPS) / total;
            assert!((v[i] + d[i] + mb + mo - 1.0).abs() < 1e-5, "bin {i}");
        }
    }

    #[test]
    fn a_dominant_net_takes_its_bin() {
        let mut v = [10.0f32];
        let mut d = [0.01f32];
        soft_masks(&mut v, &mut d, &[0.01], &[0.01]);
        assert!(v[0] > 0.999 && d[0] < 1e-4);
    }

    #[test]
    fn silence_splits_evenly_not_nan() {
        let (mut v, mut d) = ([0.0f32], [0.0f32]);
        soft_masks(&mut v, &mut d, &[0.0], &[0.0]);
        assert!((v[0] - 0.25).abs() < 1e-6 && (d[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn row_means_average_each_row() {
        let mut mask = vec![0.0f32; 2 * NET_BINS];
        mask[..NET_BINS].fill(0.5);
        mask[NET_BINS..].iter_mut().enumerate().for_each(|(i, x)| *x = (i % 2) as f32);
        let mut out = [0.0f32; 2];
        row_means(&mask, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-7 && (out[1] - 0.5).abs() < 1e-7);
    }
}
