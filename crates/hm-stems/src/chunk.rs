//! One Spleeter chunk: 512 STFT frames → the four nets → soft masks → vocals and
//! drums back through the inverse STFT. A port of the mobile engine's
//! `StemEngine::computeSplit`, independent of ONNX Runtime (the nets sit behind
//! [`NetRunner`] so the whole pipeline is testable with stand-in nets).

use realfft::num_complex::Complex32;

use crate::config::*;
use crate::mask::{row_means, soft_masks};
use crate::stft::{Stft, OLA_GAIN};
use crate::StemError;

/// A runner for the four nets. `mag` is `[2 channels][SPLIT_FRAMES][NET_BINS]`
/// and `out` receives that stem's magnitude estimate in the same layout.
pub trait NetRunner {
    fn run(&mut self, net: usize, mag: &[f32], out: &mut [f32]) -> Result<(), StemError>;
}

/// Floats in one `[2][SPLIT_FRAMES][NET_BINS]` tensor.
pub const TENSOR_LEN: usize = 2 * SPLIT_FRAMES * NET_BINS;

/// A separated owned range, interleaved stereo at [`MODEL_RATE`].
pub struct ChunkOut {
    pub vocals: Vec<f32>,
    pub drums: Vec<f32>,
}

/// Reusable working memory for [`separate_chunk`] (~30 MB).
pub struct ChunkScratch {
    stft: Stft,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    mag: Vec<f32>,
    est_v: Vec<f32>,
    est_d: Vec<f32>,
    est_tmp: Vec<f32>,
    est_bass: Vec<f32>,
    avg_v: Vec<f32>,
    avg_d: Vec<f32>,
    acc: [Vec<f32>; 4], // vocals L/R, drums L/R over the input span
}

impl Default for ChunkScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkScratch {
    pub fn new() -> Self {
        Self {
            stft: Stft::new(),
            in_l: vec![0.0; INPUT_SPAN],
            in_r: vec![0.0; INPUT_SPAN],
            mag: vec![0.0; TENSOR_LEN],
            est_v: vec![0.0; TENSOR_LEN],
            est_d: vec![0.0; TENSOR_LEN],
            est_tmp: vec![0.0; TENSOR_LEN],
            est_bass: vec![0.0; TENSOR_LEN],
            avg_v: vec![0.0; 2 * SPLIT_FRAMES],
            avg_d: vec![0.0; 2 * SPLIT_FRAMES],
            acc: std::array::from_fn(|_| vec![0.0; INPUT_SPAN]),
        }
    }
}

fn zero() -> Complex32 {
    Complex32::new(0.0, 0.0)
}

/// Separate grid chunk `c` of `mix` (interleaved stereo at 44.1 kHz). Returns
/// the owned range `owned_start(c)..min(owned_end(c), frames)`.
pub fn separate_chunk(
    runner: &mut dyn NetRunner,
    s: &mut ChunkScratch,
    mix: &[f32],
    c: usize,
) -> Result<ChunkOut, StemError> {
    let frames = mix.len() / 2;
    let own_lo = owned_start(c);
    let own_hi = owned_end(c).min(frames);
    if own_lo >= own_hi {
        return Ok(ChunkOut { vocals: Vec::new(), drums: Vec::new() });
    }

    // Gather the input span; outside the track it is silence.
    let span0 = span_start(c);
    for i in 0..INPUT_SPAN {
        let at = span0 + i as i64;
        let (l, r) = if at >= 0 && (at as usize) < frames {
            (mix[at as usize * 2], mix[at as usize * 2 + 1])
        } else {
            (0.0, 0.0)
        };
        s.in_l[i] = l;
        s.in_r[i] = r;
    }

    let mut spec_l = vec![zero(); SPEC_BINS];
    let mut spec_r = vec![zero(); SPEC_BINS];

    // Pass 1: magnitudes of the lowest NET_BINS bins.
    for j in 0..SPLIT_FRAMES {
        s.stft.forward(&s.in_l[j * HOP..j * HOP + NFFT], &mut spec_l);
        s.stft.forward(&s.in_r[j * HOP..j * HOP + NFFT], &mut spec_r);
        let row_l = j * NET_BINS;
        let row_r = (SPLIT_FRAMES + j) * NET_BINS;
        for k in 0..NET_BINS {
            s.mag[row_l + k] = spec_l[k].norm();
            s.mag[row_r + k] = spec_r[k].norm();
        }
    }

    // The soft mask needs all four estimates.
    runner.run(NET_VOCALS, &s.mag, &mut s.est_v)?;
    runner.run(NET_DRUMS, &s.mag, &mut s.est_d)?;
    runner.run(NET_BASS, &s.mag, &mut s.est_bass)?;
    runner.run(NET_OTHER, &s.mag, &mut s.est_tmp)?;
    soft_masks(&mut s.est_v, &mut s.est_d, &s.est_bass, &s.est_tmp);
    row_means(&s.est_v, &mut s.avg_v);
    row_means(&s.est_d, &mut s.avg_d);

    // Pass 2: mask the original complex spectra, inverse, overlap-add.
    for a in &mut s.acc {
        a.fill(0.0);
    }
    let mut masked = vec![zero(); SPEC_BINS];
    let mut frame = vec![0.0f32; NFFT];
    for j in 0..SPLIT_FRAMES {
        s.stft.forward(&s.in_l[j * HOP..j * HOP + NFFT], &mut spec_l);
        s.stft.forward(&s.in_r[j * HOP..j * HOP + NFFT], &mut spec_r);
        // (mask rows, per-row mean, accumulator pair) for vocals then drums.
        let stems: [(&[f32], &[f32], usize); 2] = [(&s.est_v, &s.avg_v, 0), (&s.est_d, &s.avg_d, 2)];
        for (mask, avg, acc_at) in stems {
            for (ch, spec) in [(0usize, &spec_l), (1usize, &spec_r)] {
                let row = (ch * SPLIT_FRAMES + j) * NET_BINS;
                let extension = avg[ch * SPLIT_FRAMES + j];
                for k in 0..SPEC_BINS {
                    let g = if k < NET_BINS { mask[row + k] } else { extension };
                    masked[k] = Complex32::new(spec[k].re * g, spec[k].im * g);
                }
                s.stft.inverse(&masked, &mut frame);
                for (a, f) in s.acc[acc_at + ch][j * HOP..j * HOP + NFFT].iter_mut().zip(&frame) {
                    *a += f;
                }
            }
        }
    }

    let norm = 1.0 / OLA_GAIN;
    let first = (own_lo as i64 - span0) as usize;
    let n = own_hi - own_lo;
    let mut vocals = vec![0.0f32; n * 2];
    let mut drums = vec![0.0f32; n * 2];
    for t in 0..n {
        vocals[t * 2] = s.acc[0][first + t] * norm;
        vocals[t * 2 + 1] = s.acc[1][first + t] * norm;
        drums[t * 2] = s.acc[2][first + t] * norm;
        drums[t * 2 + 1] = s.acc[3][first + t] * norm;
    }
    Ok(ChunkOut { vocals, drums })
}

#[cfg(test)]
pub(crate) mod test_nets {
    use super::*;

    /// Stand-in nets that return fixed fractions of the input magnitude, so the
    /// soft masks are known in advance: each net's mask is `w² / Σw²`.
    pub struct ScaleNets(pub [f32; NET_COUNT]);

    impl NetRunner for ScaleNets {
        fn run(&mut self, net: usize, mag: &[f32], out: &mut [f32]) -> Result<(), StemError> {
            for (o, &m) in out.iter_mut().zip(mag) {
                *o = m * self.0[net];
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_nets::ScaleNets;
    use super::*;

    fn tone(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / MODEL_RATE as f32;
                [
                    0.4 * (std::f32::consts::TAU * 440.0 * t).sin() + 0.1 * (std::f32::consts::TAU * 15_000.0 * t).sin(),
                    0.3 * (std::f32::consts::TAU * 1234.0 * t).sin() + 0.1 * (std::f32::consts::TAU * 17_000.0 * t).sin(),
                ]
            })
            .collect()
    }

    fn max_err(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn equal_nets_give_a_quarter_each_across_the_whole_spectrum() {
        // Equal nets => every mask is 1/4 in the net range AND in the extended
        // band (the average of 1/4 is 1/4), so V = D = mix/4 at all frequencies,
        // including the 15-17 kHz tones above the nets' 11 kHz range. Zeros in
        // the extension would silence those tones in V/D.
        let frames = 200_000;
        let mix = tone(frames);
        let mut s = ChunkScratch::new();
        let out = separate_chunk(&mut ScaleNets([1.0; 4]), &mut s, &mix, 0).unwrap();
        assert_eq!(out.vocals.len(), owned_end(0).min(frames) * 2);
        let quarter: Vec<f32> = mix[..out.vocals.len()].iter().map(|x| x * 0.25).collect();
        assert!(max_err(&out.vocals, &quarter) < 2e-4, "vocals {}", max_err(&out.vocals, &quarter));
        assert!(max_err(&out.drums, &quarter) < 2e-4, "drums {}", max_err(&out.drums, &quarter));
    }

    #[test]
    fn a_net_that_owns_everything_returns_the_mix() {
        // Vocals net dominant => vocals ~ mix, drums ~ 0 (masks 1 and 0).
        let frames = 150_000;
        let mix = tone(frames);
        let mut s = ChunkScratch::new();
        let out = separate_chunk(&mut ScaleNets([1000.0, 0.0, 0.0, 0.0]), &mut s, &mix, 0).unwrap();
        let n = out.vocals.len();
        assert!(max_err(&out.vocals, &mix[..n]) < 1e-3);
        assert!(out.drums.iter().all(|x| x.abs() < 1e-3));
    }

    #[test]
    fn second_chunk_continues_the_first_without_a_seam() {
        // Chunks are independent; the owned ranges must tile with no jump, i.e.
        // chunk 1's output equals a reference for the same samples.
        let frames = owned_end(1) + 5_000;
        let mix = tone(frames);
        let mut s = ChunkScratch::new();
        let nets = [1.0, 2.0, 0.5, 1.5];
        let c0 = separate_chunk(&mut ScaleNets(nets), &mut s, &mix, 0).unwrap();
        let c1 = separate_chunk(&mut ScaleNets(nets), &mut s, &mix, 1).unwrap();
        assert_eq!(c1.vocals.len(), (owned_end(1) - owned_start(1)) * 2);
        // With scale nets the masks are constant, so V == k * mix everywhere:
        let k = nets[0] * nets[0] / nets.iter().map(|w| w * w).sum::<f32>();
        let expect = |from: usize, len: usize| -> Vec<f32> { mix[from * 2..(from + len) * 2].iter().map(|x| x * k).collect() };
        assert!(max_err(&c0.vocals, &expect(0, c0.vocals.len() / 2)) < 2e-4);
        assert!(max_err(&c1.vocals, &expect(owned_start(1), c1.vocals.len() / 2)) < 2e-4);
    }

    #[test]
    fn output_is_clipped_at_the_end_of_the_track_and_empty_past_it() {
        let frames = 10_000;
        let mix = tone(frames);
        let mut s = ChunkScratch::new();
        let out = separate_chunk(&mut ScaleNets([1.0; 4]), &mut s, &mix, 0).unwrap();
        assert_eq!(out.vocals.len(), frames * 2);
        let past = separate_chunk(&mut ScaleNets([1.0; 4]), &mut s, &mix, 3).unwrap();
        assert!(past.vocals.is_empty() && past.drums.is_empty());
    }
}
