//! Real-time multi-stem playback — the mixing side of stem separation.
//!
//! Demucs separates a track **offline** into four stems (vocals, drums, bass,
//! other); [`StemPlaybackSource`] then plays them back **in sync**, summing them
//! with live, per-stem gains so the UI's faders / mute / solo are instant. It's
//! an [`AudioSource`](crate::AudioSource), so it drops into the engine exactly
//! like normal file playback — the DSP chain still applies to the mixed result.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// The four Demucs stems, in a fixed order.
pub const STEM_COUNT: usize = 4;
pub const STEM_VOCALS: usize = 0;
pub const STEM_DRUMS: usize = 1;
pub const STEM_BASS: usize = 2;
pub const STEM_OTHER: usize = 3;

/// Live, lock-free per-stem gains shared between the UI (writer) and the audio
/// thread (reader). Each gain is an `f32` stored in an `AtomicU32`.
pub struct StemGains {
    gains: [AtomicU32; STEM_COUNT],
}

impl Default for StemGains {
    fn default() -> Self {
        Self {
            gains: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
        }
    }
}

impl StemGains {
    pub fn set(&self, stem: usize, gain: f32) {
        if stem < STEM_COUNT {
            self.gains[stem].store(gain.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn get(&self, stem: usize) -> f32 {
        if stem < STEM_COUNT {
            f32::from_bits(self.gains[stem].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    fn snapshot(&self) -> [f32; STEM_COUNT] {
        [self.get(0), self.get(1), self.get(2), self.get(3)]
    }
}

/// Per-frame smoothing toward the target gain (~5 ms at 48 kHz) so moving a
/// fader doesn't click ("zipper noise").
const GAIN_SMOOTH: f32 = 0.0025;

/// Plays four pre-decoded, interleaved **stereo** stems (all at the engine rate)
/// mixed by live gains. `read` is allocation-free and real-time safe.
pub struct StemPlaybackSource {
    stems: [Vec<f32>; STEM_COUNT],
    /// Frame count of each stem; an empty stem (2-stem mode leaves two empty)
    /// simply contributes silence rather than truncating playback.
    stem_frames: [usize; STEM_COUNT],
    gains: Arc<StemGains>,
    smoothed: [f32; STEM_COUNT],
    cursor: usize,
    len_frames: usize,
}

impl StemPlaybackSource {
    /// `stems` are interleaved-stereo buffers at the target rate; playback runs
    /// to the longest one (empty stems are skipped). `gains` is shared with the
    /// engine for live faders.
    pub fn new(stems: [Vec<f32>; STEM_COUNT], gains: Arc<StemGains>) -> Self {
        let stem_frames = [
            stems[0].len() / 2,
            stems[1].len() / 2,
            stems[2].len() / 2,
            stems[3].len() / 2,
        ];
        let len_frames = stem_frames.iter().copied().max().unwrap_or(0);
        let smoothed = gains.snapshot();
        Self {
            stems,
            stem_frames,
            gains,
            smoothed,
            cursor: 0,
            len_frames,
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
            // Glide each stem's gain toward its target this frame.
            for s in 0..STEM_COUNT {
                self.smoothed[s] += (target[s] - self.smoothed[s]) * GAIN_SMOOTH;
            }

            if self.cursor < self.len_frames {
                let i = self.cursor * 2;
                let mut l = 0.0f32;
                let mut r = 0.0f32;
                for s in 0..STEM_COUNT {
                    // Skip stems that are empty or have already ended.
                    if self.cursor >= self.stem_frames[s] {
                        continue;
                    }
                    let g = self.smoothed[s];
                    l += self.stems[s][i] * g;
                    r += self.stems[s][i + 1] * g;
                }
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
        // Run enough silence-free frames for the smoothing to converge.
        let mut sink = vec![0.0f32; frames * 2];
        src.read(&mut sink, 2);
    }

    #[test]
    fn mutes_a_stem_via_gain() {
        // vocals = +1 const, drums = -1 const, others silent.
        let n = 4096;
        let vocals = vec![1.0f32; n * 2];
        let drums = vec![-1.0f32; n * 2];
        let bass = vec![0.0f32; n * 2];
        let other = vec![0.0f32; n * 2];
        let gains = Arc::new(StemGains::default());
        gains.set(STEM_VOCALS, 1.0);
        gains.set(STEM_DRUMS, 0.0); // mute drums
        let mut src = StemPlaybackSource::new([vocals, drums, bass, other], gains);

        settle(&mut src, 2000); // let the gain glide settle
        let mut out = vec![0.0f32; 2];
        src.read(&mut out, 2);
        // Only vocals (gain 1) should remain → ~+1.0, drums muted out.
        assert!((out[0] - 1.0).abs() < 0.02, "got {}", out[0]);
        assert!((out[1] - 1.0).abs() < 0.02, "got {}", out[1]);
    }

    #[test]
    fn plays_to_longest_stem_skipping_empty_ones() {
        // 2-stem shape: vocals + instrumental present, drums/bass empty.
        let gains = Arc::new(StemGains::default());
        let src_stems = [
            vec![1.0, 1.0, 1.0, 1.0], // vocals: 2 frames
            vec![],                   // drums: empty
            vec![],                   // bass: empty
            vec![0.5, 0.5],           // instrumental: 1 frame
        ];
        let mut src = StemPlaybackSource::new(src_stems, gains);
        assert_eq!(src.total_frames(), 2); // longest
        let mut out = vec![0.0f32; 8]; // 4 frames requested
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 2);
        // Frame 0: vocals(1) + instrumental(0.5) = 1.5; frame 1: vocals only = 1.0.
        assert!((out[0] - 1.5).abs() < 0.01, "f0 = {}", out[0]);
        assert!((out[2] - 1.0).abs() < 0.01, "f1 = {}", out[2]);
    }
}
