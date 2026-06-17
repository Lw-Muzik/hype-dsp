//! Look-ahead brickwall limiter.
//!
//! A delay line lets the gain reduction react *before* a peak reaches the
//! output. Gain reduction is applied instantly (clamped to the window peak) and
//! released smoothly, which guarantees the output never exceeds the ceiling
//! while staying transparent on quiet passages. Gain is linked across channels
//! so stereo imaging is preserved.

use crate::{AudioProcessor, ProcessorParams};

/// Look-ahead window in milliseconds.
const LOOKAHEAD_MS: f32 = 5.0;
/// Gain release time constant in milliseconds.
const RELEASE_MS: f32 = 100.0;

/// A peak brickwall limiter with look-ahead.
pub struct Limiter {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    ceiling_lin: f32,
    /// Delay line of interleaved frames; length = lookahead_frames * channels.
    delay: Vec<f32>,
    write_idx: usize,
    lookahead_frames: usize,
    release_coeff: f32,
    gain: f32,
}

impl Limiter {
    /// Create a limiter prepared for the given format with a safe default
    /// ceiling.
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut lim = Self {
            sample_rate,
            channels,
            enabled: true,
            ceiling_lin: 0.966, // ≈ -0.3 dBFS
            delay: Vec::new(),
            write_idx: 0,
            lookahead_frames: 0,
            release_coeff: 0.0,
            gain: 1.0,
        };
        lim.reconfigure();
        lim
    }

    /// Size the delay line and release coefficient for the current format.
    fn reconfigure(&mut self) {
        let channels = self.channels.max(1);
        self.lookahead_frames = ((LOOKAHEAD_MS / 1000.0) * self.sample_rate)
            .round()
            .max(1.0) as usize;
        self.delay = vec![0.0; self.lookahead_frames * channels];
        self.write_idx = 0;
        self.gain = 1.0;
        let tau = (RELEASE_MS / 1000.0) * self.sample_rate;
        self.release_coeff = 1.0 - (-1.0 / tau).exp();
    }
}

impl AudioProcessor for Limiter {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 {
            return;
        }
        // Adapt if the stream channel count differs from preparation.
        if channels != self.channels {
            self.channels = channels;
            self.reconfigure();
        }
        if self.lookahead_frames == 0 {
            return;
        }

        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;

            // Peak across the look-ahead window: the queued delay-line frames
            // (which include the frame about to be output) plus the incoming
            // frame. Linking channels keeps the stereo image stable.
            let mut window_peak = 0.0f32;
            for &queued in self.delay.iter() {
                let a = queued.abs();
                if a > window_peak {
                    window_peak = a;
                }
            }
            for ch in 0..channels {
                let a = buffer[base + ch].abs();
                if a > window_peak {
                    window_peak = a;
                }
            }

            let target = if window_peak > self.ceiling_lin {
                self.ceiling_lin / window_peak
            } else {
                1.0
            };
            // Smooth release toward unity, instant attack clamped to target so
            // the output can never overshoot the ceiling.
            self.gain += (1.0 - self.gain) * self.release_coeff;
            if self.gain > target {
                self.gain = target;
            }

            let slot = self.write_idx * channels;
            for ch in 0..channels {
                let out = self.delay[slot + ch] * self.gain;
                self.delay[slot + ch] = buffer[base + ch];
                buffer[base + ch] = out;
            }
            self.write_idx = (self.write_idx + 1) % self.lookahead_frames;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.output.limiter_enabled;
        self.ceiling_lin = 10f32.powf(params.output.ceiling_db / 20.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, OutputState};

    /// Under a hot input the output peak must never exceed the ceiling.
    #[test]
    fn holds_output_below_ceiling() {
        let sample_rate = 48_000.0;
        let state = EngineState {
            output: OutputState {
                ceiling_db: -0.3,
                ..Default::default()
            },
            ..Default::default()
        };
        let ceiling_lin = 10f32.powf(-0.3 / 20.0);

        let mut lim = Limiter::new(sample_rate, 2);
        lim.set_params(&state);

        // Hot stereo sine at amplitude 4.0 (≈ +12 dBFS).
        let frames = 48_000;
        let mut buffer = vec![0.0f32; frames * 2];
        for f in 0..frames {
            let s = (f as f32 / sample_rate * 2.0 * std::f32::consts::PI * 220.0).sin() * 4.0;
            buffer[f * 2] = s;
            buffer[f * 2 + 1] = s;
        }

        lim.process(&mut buffer, 2);

        let peak = buffer.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(
            peak <= ceiling_lin + 1e-3,
            "peak {peak} exceeded ceiling {ceiling_lin}"
        );
    }

    /// A quiet signal (already under the ceiling) passes through with unity
    /// gain once the look-ahead delay has flushed.
    #[test]
    fn transparent_below_ceiling() {
        let sample_rate = 48_000.0;
        let mut lim = Limiter::new(sample_rate, 1);
        lim.set_params(&EngineState::default());

        let frames = 4_000;
        let input: Vec<f32> = (0..frames)
            .map(|f| (f as f32 / sample_rate * 2.0 * std::f32::consts::PI * 440.0).sin() * 0.2)
            .collect();
        let mut buffer = input.clone();
        lim.process(&mut buffer, 1);

        // Compare past the look-ahead latency; signal should be unchanged.
        let latency = (LOOKAHEAD_MS / 1000.0 * sample_rate) as usize;
        for i in (latency + 100)..frames {
            assert!(
                (buffer[i] - input[i - latency]).abs() < 1e-3,
                "sample {i}: {} vs {}",
                buffer[i],
                input[i - latency]
            );
        }
    }
}
