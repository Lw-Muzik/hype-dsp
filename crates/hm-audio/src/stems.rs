//! Real-time multi-stem playback — the mixing side of stem separation.
//!
//! htdemucs separates a track into four buffers (vocals, drums, bass, other);
//! [`StemPlaybackSource`] plays them back **in sync**, but exposes **five live,
//! controllable elements** to match VirtualDJ's pad grid: the drum buffer is
//! split in the audio thread by a complementary crossover into **Kick** (lows)
//! and **HiHat** (highs). So the elements are:
//!
//! | element | index | source |
//! |---------|-------|--------|
//! | Vocals  | 0 | vocals buffer |
//! | Kick    | 1 | drums buffer, low band |
//! | HiHat   | 2 | drums buffer, high band |
//! | Bass    | 3 | bass buffer |
//! | Melody  | 4 | other buffer |
//!
//! The UI's "Instru / Acapella / Instrument" pads are just mute/solo groups over
//! these. Per-element gains are live, lock-free, and smoothed (no zipper noise).
//! It's an [`AudioSource`](crate::AudioSource), so it drops into the engine like
//! normal file playback and the DSP chain still applies to the mixed result.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// Number of separated input buffers (vocals, drums, bass, other).
pub const STEM_COUNT: usize = 4;

/// Number of live, controllable mix elements (drums split into kick + hihat).
pub const ELEMENT_COUNT: usize = 5;
pub const EL_VOCALS: usize = 0;
pub const EL_KICK: usize = 1;
pub const EL_HIHAT: usize = 2;
pub const EL_BASS: usize = 3;
pub const EL_MELODY: usize = 4;

/// Input-buffer indices (what htdemucs produces).
const BUF_VOCALS: usize = 0;
const BUF_DRUMS: usize = 1;
const BUF_BASS: usize = 2;
const BUF_OTHER: usize = 3;

/// Kick/HiHat crossover on the isolated drum stem. Below = Kick, above = HiHat.
const CROSSOVER_HZ: f32 = 150.0;

/// Live, lock-free per-element gains shared between the UI (writer) and the
/// audio thread (reader). Each gain is an `f32` stored in an `AtomicU32`.
pub struct StemGains {
    gains: [AtomicU32; ELEMENT_COUNT],
}

impl Default for StemGains {
    fn default() -> Self {
        Self {
            gains: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
        }
    }
}

impl StemGains {
    pub fn set(&self, element: usize, gain: f32) {
        if element < ELEMENT_COUNT {
            self.gains[element].store(gain.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn get(&self, element: usize) -> f32 {
        if element < ELEMENT_COUNT {
            f32::from_bits(self.gains[element].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    fn snapshot(&self) -> [f32; ELEMENT_COUNT] {
        std::array::from_fn(|i| self.get(i))
    }
}

/// Per-frame smoothing toward the target gain (~5 ms at 48 kHz) so toggling a
/// pad doesn't click ("zipper noise").
const GAIN_SMOOTH: f32 = 0.0025;

/// A second-order (RBJ) biquad lowpass; `process` returns the low band, the
/// complementary high band is `input - low` (reconstructs exactly at unity).
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn lowpass(sample_rate: f32, fc: f32, q: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * fc / sample_rate.max(1.0);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let mut y =
            self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        if y.is_subnormal() {
            y = 0.0;
        }
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Plays four pre-decoded, interleaved **stereo** stems (all at the engine rate),
/// splitting drums into kick/hihat and mixing all five elements by live gains.
/// `read` is allocation-free and real-time safe.
pub struct StemPlaybackSource {
    stems: [Vec<f32>; STEM_COUNT],
    /// Frame count of each buffer; an empty buffer simply contributes silence.
    stem_frames: [usize; STEM_COUNT],
    gains: Arc<StemGains>,
    smoothed: [f32; ELEMENT_COUNT],
    /// Drum crossover lowpass, one per channel (L, R).
    drum_lp: [Biquad; 2],
    cursor: usize,
    len_frames: usize,
}

impl StemPlaybackSource {
    /// `stems` are interleaved-stereo buffers at the target rate (in buffer order
    /// vocals, drums, bass, other); playback runs to the longest. `gains` is
    /// shared with the engine for live pads. `sample_rate` sets the kick/hihat
    /// crossover.
    pub fn new(stems: [Vec<f32>; STEM_COUNT], gains: Arc<StemGains>, sample_rate: f32) -> Self {
        let stem_frames = std::array::from_fn(|i| stems[i].len() / 2);
        let len_frames = stem_frames.iter().copied().max().unwrap_or(0);
        let smoothed = gains.snapshot();
        let lp = Biquad::lowpass(sample_rate, CROSSOVER_HZ, std::f32::consts::FRAC_1_SQRT_2);
        Self {
            stems,
            stem_frames,
            gains,
            smoothed,
            drum_lp: [lp, lp],
            cursor: 0,
            len_frames,
        }
    }

    /// Sample from buffer `b` at interleaved index `idx`, or 0 past its end.
    #[inline]
    fn sample(&self, b: usize, idx: usize) -> f32 {
        if self.cursor < self.stem_frames[b] {
            self.stems[b][idx]
        } else {
            0.0
        }
    }
}

impl AudioSource for StemPlaybackSource {
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        let target = self.gains.snapshot();
        let out_frames = out.len() / channels;
        let mut produced = 0;

        for f in 0..out_frames {
            let base = f * channels;
            for (sm, &t) in self.smoothed.iter_mut().zip(target.iter()) {
                *sm += (t - *sm) * GAIN_SMOOTH;
            }

            if self.cursor < self.len_frames {
                let i = self.cursor * 2;
                let g = &self.smoothed;

                // Drums → kick (low) + hihat (high), per channel.
                let dl = self.sample(BUF_DRUMS, i);
                let dr = self.sample(BUF_DRUMS, i + 1);
                let kick_l = self.drum_lp[0].process(dl);
                let kick_r = self.drum_lp[1].process(dr);
                let hat_l = dl - kick_l;
                let hat_r = dr - kick_r;

                let l = self.sample(BUF_VOCALS, i) * g[EL_VOCALS]
                    + kick_l * g[EL_KICK]
                    + hat_l * g[EL_HIHAT]
                    + self.sample(BUF_BASS, i) * g[EL_BASS]
                    + self.sample(BUF_OTHER, i) * g[EL_MELODY];
                let r = self.sample(BUF_VOCALS, i + 1) * g[EL_VOCALS]
                    + kick_r * g[EL_KICK]
                    + hat_r * g[EL_HIHAT]
                    + self.sample(BUF_BASS, i + 1) * g[EL_BASS]
                    + self.sample(BUF_OTHER, i + 1) * g[EL_MELODY];

                self.cursor += 1;
                produced += 1;
                if channels == 1 {
                    out[base] = 0.5 * (l + r);
                } else {
                    out[base] = l;
                    out[base + 1] = r;
                    for ch in out.iter_mut().take(base + channels).skip(base + 2) {
                        *ch = 0.0;
                    }
                }
            } else {
                for ch in out.iter_mut().take(base + channels).skip(base) {
                    *ch = 0.0;
                }
            }
        }
        produced
    }

    fn stop(&mut self) {
        self.cursor = self.len_frames;
    }

    fn seek(&mut self, frame: usize) {
        self.cursor = frame.min(self.len_frames);
    }

    fn position(&self) -> usize {
        self.cursor.min(self.len_frames)
    }

    fn total_frames(&self) -> usize {
        self.len_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(src: &mut StemPlaybackSource, frames: usize) {
        // Run enough frames for the gain smoothing + filter to converge.
        let mut sink = vec![0.0f32; frames * 2];
        src.read(&mut sink, 2);
    }

    #[test]
    fn mutes_an_element_via_gain() {
        // vocals = +1 const; drums = +0.5 const (DC → all in the kick band).
        let n = 8192;
        let gains = Arc::new(StemGains::default());
        gains.set(EL_KICK, 0.0); // mute kick → the DC drum disappears
        let mut src = StemPlaybackSource::new(
            [
                vec![1.0f32; n * 2], // vocals
                vec![0.5f32; n * 2], // drums (DC)
                vec![0.0f32; n * 2], // bass
                vec![0.0f32; n * 2], // other
            ],
            gains,
            48_000.0,
        );

        settle(&mut src, 4000);
        let mut out = vec![0.0f32; 2];
        src.read(&mut out, 2);
        // Drums (DC) routed entirely to kick, which is muted → only vocals (1).
        assert!((out[0] - 1.0).abs() < 0.02, "got {}", out[0]);
        assert!((out[1] - 1.0).abs() < 0.02, "got {}", out[1]);
    }

    #[test]
    fn drum_bands_reconstruct_at_unity() {
        // Only drums present; kick+hihat at unity must sum back to the drum.
        let n = 8192;
        let gains = Arc::new(StemGains::default());
        gains.set(EL_VOCALS, 0.0);
        gains.set(EL_BASS, 0.0);
        gains.set(EL_MELODY, 0.0);
        let mut src = StemPlaybackSource::new(
            [
                vec![0.0f32; n * 2],
                vec![0.3f32; n * 2], // drums (DC)
                vec![0.0f32; n * 2],
                vec![0.0f32; n * 2],
            ],
            gains,
            48_000.0,
        );
        settle(&mut src, 4000);
        let mut out = vec![0.0f32; 2];
        src.read(&mut out, 2);
        assert!((out[0] - 0.3).abs() < 0.01, "got {}", out[0]);
    }

    #[test]
    fn plays_to_longest_stem_skipping_empty_ones() {
        // vocals + other present, drums/bass empty.
        let gains = Arc::new(StemGains::default());
        let src_stems = [
            vec![1.0, 1.0, 1.0, 1.0], // vocals: 2 frames
            vec![],                   // drums: empty
            vec![],                   // bass: empty
            vec![0.5, 0.5],           // other/melody: 1 frame
        ];
        let mut src = StemPlaybackSource::new(src_stems, gains, 48_000.0);
        assert_eq!(src.total_frames(), 2); // longest
        let mut out = vec![0.0f32; 8]; // 4 frames requested
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 2);
        // Frame 0: vocals(1) + melody(0.5) = 1.5; frame 1: vocals only = 1.0.
        assert!((out[0] - 1.5).abs() < 0.01, "f0 = {}", out[0]);
        assert!((out[2] - 1.0).abs() < 0.01, "f1 = {}", out[2]);
    }
}
