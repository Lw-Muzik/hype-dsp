//! Look-ahead brickwall limiter.
//!
//! A delay line lets the gain reduction react *before* a peak reaches the
//! output. Gain reduction is applied instantly (clamped to the window peak) and
//! released smoothly, which guarantees the output never exceeds the ceiling
//! while staying transparent on quiet passages. Gain is linked across channels
//! so stereo imaging is preserved.

use crate::{AudioProcessor, ProcessorParams};
use std::collections::VecDeque;

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
    /// Monotonic deque of `(frame index, frame peak)` for an O(1)-amortised
    /// sliding-window maximum over the look-ahead window. Peaks are strictly
    /// decreasing front→back, so the front is always the exact window maximum
    /// (identical to rescanning the whole delay line every frame). Pre-sized in
    /// `reconfigure`; never reallocates in `process`.
    window_max: VecDeque<(u64, f32)>,
    /// Monotonic frame counter driving the deque's window eviction.
    frame_counter: u64,
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
            window_max: VecDeque::new(),
            frame_counter: 0,
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
        // The monotonic deque holds at most one entry per frame in the window
        // (lookahead_frames queued + the incoming frame), so this capacity is
        // never exceeded and `process` never reallocates.
        self.window_max = VecDeque::with_capacity(self.lookahead_frames + 1);
        self.frame_counter = 0;
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

            // Peak of the incoming frame. Linking channels keeps the stereo
            // image stable.
            let mut frame_peak = 0.0f32;
            for ch in 0..channels {
                let a = buffer[base + ch].abs();
                if a > frame_peak {
                    frame_peak = a;
                }
            }

            // Peak across the look-ahead window: the queued delay-line frames
            // (which include the frame about to be output) plus the incoming
            // frame — i.e. frames [idx − lookahead, idx]. The monotonic deque
            // yields the exact same maximum as rescanning the whole delay line
            // (the delay line's initial zeros never win because frame peaks
            // are non-negative), in O(1) amortised per frame.
            let idx = self.frame_counter;
            self.frame_counter += 1;
            let oldest = idx.saturating_sub(self.lookahead_frames as u64);
            // Evict frames that slid out of the window.
            while let Some(&(i, _)) = self.window_max.front() {
                if i >= oldest {
                    break;
                }
                self.window_max.pop_front();
            }
            // Drop peaks dominated by the incoming frame (they can never be
            // the maximum again), keeping the deque decreasing front→back.
            while let Some(&(_, p)) = self.window_max.back() {
                if p > frame_peak {
                    break;
                }
                self.window_max.pop_back();
            }
            self.window_max.push_back((idx, frame_peak));
            let window_peak = self.window_max[0].1;

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
            self.write_idx += 1;
            if self.write_idx == self.lookahead_frames {
                self.write_idx = 0;
            }
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

    /// Reference implementation: the original O(frames × window) full rescan
    /// of the delay line, kept verbatim so the deque-based sliding-window
    /// maximum can be proven bit-identical to it.
    struct NaiveLimiter {
        ceiling_lin: f32,
        delay: Vec<f32>,
        write_idx: usize,
        lookahead_frames: usize,
        release_coeff: f32,
        gain: f32,
    }

    impl NaiveLimiter {
        fn new(sample_rate: f32, channels: usize, ceiling_db: f32) -> Self {
            let lookahead_frames =
                ((LOOKAHEAD_MS / 1000.0) * sample_rate).round().max(1.0) as usize;
            let tau = (RELEASE_MS / 1000.0) * sample_rate;
            Self {
                ceiling_lin: 10f32.powf(ceiling_db / 20.0),
                delay: vec![0.0; lookahead_frames * channels.max(1)],
                write_idx: 0,
                lookahead_frames,
                release_coeff: 1.0 - (-1.0 / tau).exp(),
                gain: 1.0,
            }
        }

        fn process(&mut self, buffer: &mut [f32], channels: usize) {
            let frames = buffer.len() / channels;
            for f in 0..frames {
                let base = f * channels;
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
    }

    /// Run the same signal through the deque limiter (in uneven chunks, to
    /// exercise cross-block deque state) and the naive rescanning reference,
    /// and require bit-identical output.
    fn assert_matches_naive(signal: &[f32], channels: usize, label: &str) {
        let sample_rate = 48_000.0;
        let ceiling_db = -0.3;
        let state = EngineState {
            output: OutputState {
                ceiling_db,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut fast = Limiter::new(sample_rate, channels);
        fast.set_params(&state);
        let mut fast_buf = signal.to_vec();
        // Odd chunk sizes so the deque state must survive block boundaries.
        let mut off = 0;
        for chunk in [37usize, 480, 1, 999, 256].iter().cycle() {
            if off >= fast_buf.len() {
                break;
            }
            let end = (off + chunk * channels).min(fast_buf.len());
            fast.process(&mut fast_buf[off..end], channels);
            off = end;
        }

        let mut naive = NaiveLimiter::new(sample_rate, channels, ceiling_db);
        let mut naive_buf = signal.to_vec();
        naive.process(&mut naive_buf, channels);

        for (i, (&a, &b)) in fast_buf.iter().zip(naive_buf.iter()).enumerate() {
            assert!(
                a.to_bits() == b.to_bits(),
                "{label}: sample {i} differs: deque={a} naive={b}"
            );
        }
    }

    /// Deque limiter must be bit-identical to the naive rescan on an impulse.
    #[test]
    fn matches_naive_on_impulse() {
        let mut signal = vec![0.0f32; 4_000 * 2];
        signal[100 * 2] = 4.0;
        signal[100 * 2 + 1] = -4.0;
        signal[2_500 * 2] = 2.0; // second spike after the release has run
        assert_matches_naive(&signal, 2, "impulse");
    }

    /// Deque limiter must be bit-identical to the naive rescan on a hot sine
    /// burst followed by silence (attack, hold, and full release paths).
    #[test]
    fn matches_naive_on_sine_burst() {
        let sample_rate = 48_000.0;
        let burst = 6_000;
        let mut signal: Vec<f32> = (0..burst)
            .flat_map(|f| {
                let s =
                    (f as f32 / sample_rate * 2.0 * std::f32::consts::PI * 220.0).sin() * 2.0;
                [s, s * 0.8]
            })
            .collect();
        signal.extend(std::iter::repeat_n(0.0, 6_000 * 2));
        assert_matches_naive(&signal, 2, "sine burst");
    }

    /// Deque limiter must be bit-identical to the naive rescan on noise
    /// (deterministic LCG so the test is reproducible).
    #[test]
    fn matches_naive_on_noise() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 * 6.0 - 3.0 // ∈ [-3, 3)
        };
        let signal: Vec<f32> = (0..12_000 * 2).map(|_| next()).collect();
        assert_matches_naive(&signal, 2, "noise");
        let mono: Vec<f32> = (0..12_000).map(|_| next()).collect();
        assert_matches_naive(&mono, 1, "noise mono");
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
