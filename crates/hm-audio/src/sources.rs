//! Concrete [`AudioSource`] implementations.

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// Plays pre-decoded, interleaved **stereo** samples (at the engine/device
/// sample rate). All decoding and resampling happens off the audio thread; this
/// source just copies frames, so `read` is allocation-free and real-time safe.
pub struct FilePlaybackSource {
    samples: Vec<f32>,
    /// Frame cursor (each frame is one L/R pair).
    cursor: usize,
}

impl FilePlaybackSource {
    /// Create a source over interleaved stereo samples at the target rate.
    pub fn new(stereo_samples: Vec<f32>) -> Self {
        Self {
            samples: stereo_samples,
            cursor: 0,
        }
    }

    fn len_frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Whether playback has reached the end.
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.len_frames()
    }
}

impl AudioSource for FilePlaybackSource {
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        let total = self.len_frames();
        let out_frames = out.len() / channels;
        let mut produced = 0;
        for f in 0..out_frames {
            let base = f * channels;
            if self.cursor < total {
                let l = self.samples[self.cursor * 2];
                let r = self.samples[self.cursor * 2 + 1];
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
        self.cursor = self.len_frames();
    }

    fn seek(&mut self, frame: usize) {
        self.cursor = frame.min(self.len_frames());
    }

    fn position(&self) -> usize {
        self.cursor.min(self.len_frames())
    }

    fn total_frames(&self) -> usize {
        self.len_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stereo_frames_then_silence() {
        // 2 frames of audio: (0.5,-0.5), (0.25,-0.25).
        let mut src = FilePlaybackSource::new(vec![0.5, -0.5, 0.25, -0.25]);
        let mut out = vec![0.0f32; 8]; // 4 stereo frames requested
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 2);
        assert_eq!(&out[0..4], &[0.5, -0.5, 0.25, -0.25]);
        assert_eq!(&out[4..8], &[0.0, 0.0, 0.0, 0.0]); // silence past EOF
        assert!(src.is_finished());
    }

    #[test]
    fn downmixes_to_mono() {
        let mut src = FilePlaybackSource::new(vec![0.5, -0.5]);
        let mut out = vec![0.0f32; 1];
        let produced = src.read(&mut out, 1);
        assert_eq!(produced, 1);
        assert!((out[0] - 0.0).abs() < 1e-6); // 0.5 + -0.5 averaged = 0
    }
}
