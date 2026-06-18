//! Streaming linear resampler for interleaved stereo.
//!
//! Pulls input frames on demand (so it can sit on top of a lock-free ring) and
//! emits one output frame per call. `step` = input frames consumed per output
//! frame = `in_rate / out_rate`. Used by the system-audio tap to convert the
//! tap's capture rate to the output device rate (e.g. 48000 -> 44100), avoiding
//! the pitch/tempo shift and ring saturation that a raw rate mismatch causes.

/// Streaming linear resampler. Stereo, frame-at-a-time, pull-based.
pub struct StereoResampler {
    /// Input frames consumed per output frame (`in_rate / out_rate`).
    step: f64,
    cur: (f32, f32),
    nxt: (f32, f32),
    frac: f64,
    primed: bool,
}

impl Default for StereoResampler {
    fn default() -> Self {
        Self {
            step: 1.0,
            cur: (0.0, 0.0),
            nxt: (0.0, 0.0),
            frac: 0.0,
            primed: false,
        }
    }
}

impl StereoResampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the conversion ratio and reset interpolation state. A 1:1 ratio is a
    /// transparent pass-through (still linearly interpolates at integer points).
    pub fn set_ratio(&mut self, in_rate: u32, out_rate: u32) {
        self.step = f64::from(in_rate) / f64::from(out_rate.max(1));
        self.frac = 0.0;
        self.primed = false;
    }

    /// Produce the next output frame, pulling input frames via `pull` (which
    /// returns `None` on underflow). Returns `None` only if the very first
    /// priming pulls underflow; after priming it holds the last frame on
    /// underflow so the stream never stalls hard.
    pub fn next_frame<F>(&mut self, mut pull: F) -> Option<(f32, f32)>
    where
        F: FnMut() -> Option<(f32, f32)>,
    {
        if !self.primed {
            self.cur = pull()?;
            self.nxt = pull()?;
            self.frac = 0.0;
            self.primed = true;
        }

        let t = self.frac as f32;
        let out = (
            self.cur.0 + (self.nxt.0 - self.cur.0) * t,
            self.cur.1 + (self.nxt.1 - self.cur.1) * t,
        );

        self.frac += self.step;
        while self.frac >= 1.0 {
            match pull() {
                Some(f) => {
                    self.cur = self.nxt;
                    self.nxt = f;
                    self.frac -= 1.0;
                }
                None => {
                    // Underflow: hold the last frame until more audio arrives.
                    self.cur = self.nxt;
                    self.frac = 0.0;
                    break;
                }
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linear interpolation of a ramp is exact, so output frame k must equal the
    /// input sampled at position k*step — proving correct rate conversion.
    #[test]
    fn resamples_ramp_at_correct_rate() {
        let mut rs = StereoResampler::new();
        rs.set_ratio(48_000, 44_100);
        let input: Vec<(f32, f32)> = (0..4000).map(|i| (i as f32, -(i as f32))).collect();
        let step = 48_000.0_f64 / 44_100.0_f64;

        let mut idx = 0usize;
        for k in 0..2000 {
            let frame = rs
                .next_frame(|| {
                    let v = input.get(idx).copied();
                    idx += 1;
                    v
                })
                .expect("no underflow with ample input");
            let expected = (k as f64 * step) as f32;
            assert!(
                (frame.0 - expected).abs() < 0.01,
                "k={k}: L got {} want {expected}",
                frame.0
            );
            assert!(
                (frame.1 + expected).abs() < 0.01,
                "k={k}: R got {} want {}",
                frame.1,
                -expected
            );
        }
    }

    /// 1:1 ratio reproduces the input exactly at every frame.
    #[test]
    fn unity_ratio_is_passthrough() {
        let mut rs = StereoResampler::new();
        rs.set_ratio(48_000, 48_000);
        let input: Vec<(f32, f32)> = (0..100).map(|i| (i as f32 * 0.5, i as f32 * 0.25)).collect();
        let mut idx = 0usize;
        for k in 0..50 {
            let frame = rs
                .next_frame(|| {
                    let v = input.get(idx).copied();
                    idx += 1;
                    v
                })
                .unwrap();
            assert!((frame.0 - input[k].0).abs() < 1e-5, "k={k} L");
            assert!((frame.1 - input[k].1).abs() < 1e-5, "k={k} R");
        }
    }

    /// Consumes input at the capture rate: ~step input frames per output frame.
    #[test]
    fn consumes_input_at_capture_rate() {
        let mut rs = StereoResampler::new();
        rs.set_ratio(48_000, 44_100);
        let input: Vec<(f32, f32)> = vec![(0.3, -0.3); 10_000];
        let mut idx = 0usize;
        let out_frames = 4410; // 0.1 s of output
        for _ in 0..out_frames {
            let _ = rs.next_frame(|| {
                let v = input.get(idx).copied();
                idx += 1;
                v
            });
        }
        // ~4800 input frames consumed for 4410 output frames (48k vs 44.1k),
        // plus a couple for priming. Allow a small slack.
        let expected = (out_frames as f64 * 48_000.0 / 44_100.0) as usize;
        assert!(
            idx >= expected && idx <= expected + 4,
            "consumed {idx}, expected ~{expected}"
        );
    }
}
