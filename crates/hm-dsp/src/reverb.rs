//! A compact Freeverb-style room reverb: parallel damped comb filters feeding
//! series allpass diffusers, producing a **decorrelated stereo** field from a
//! mono input. Used by the 3D-surround stage to turn the rear speakers into a
//! large, enveloping room rather than a static widening.
//!
//! Stability: every feedback path has gain < 1, so the response always decays.

/// Comb delay tunings at 44.1 kHz (Freeverb's first four), scaled to the actual
/// sample rate at construction.
const COMB_TUNINGS: [usize; 4] = [1116, 1188, 1277, 1356];
/// Allpass diffuser tunings at 44.1 kHz.
const ALLPASS_TUNINGS: [usize; 2] = [556, 441];
/// Right-channel delay offset (samples @44.1 kHz) for L/R decorrelation.
const STEREO_SPREAD: usize = 23;
/// Comb feedback (room size). < 1 ⇒ the tail decays. ~0.86 ≈ a large room.
const COMB_FEEDBACK: f32 = 0.86;
/// Damping: how fast highs die away in the tail (0 = bright, 1 = dark).
const DAMP: f32 = 0.22;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Input attenuation so the summed combs don't overload the tail.
const INPUT_GAIN: f32 = 0.025;

/// A damped feedback comb filter.
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            store: 0.0,
            feedback: COMB_FEEDBACK,
            damp1: DAMP,
            damp2: 1.0 - DAMP,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let out = self.buf[self.idx];
        // One-pole damping low-pass inside the feedback loop.
        self.store = out * self.damp2 + self.store * self.damp1;
        self.buf[self.idx] = x + self.store * self.feedback;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

/// A Schroeder allpass diffuser.
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let buffed = self.buf[self.idx];
        let out = -x + buffed;
        self.buf[self.idx] = x + buffed * ALLPASS_FEEDBACK;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

/// Mono-in, decorrelated-stereo-out room reverb.
pub(crate) struct RoomReverb {
    comb_l: Vec<Comb>,
    comb_r: Vec<Comb>,
    ap_l: Vec<Allpass>,
    ap_r: Vec<Allpass>,
}

impl RoomReverb {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let scale = |n: usize| ((n as f32) * sample_rate / 44_100.0).round().max(1.0) as usize;
        let spread = scale(STEREO_SPREAD);
        RoomReverb {
            comb_l: COMB_TUNINGS.iter().map(|&t| Comb::new(scale(t))).collect(),
            comb_r: COMB_TUNINGS
                .iter()
                .map(|&t| Comb::new(scale(t) + spread))
                .collect(),
            ap_l: ALLPASS_TUNINGS.iter().map(|&t| Allpass::new(scale(t))).collect(),
            ap_r: ALLPASS_TUNINGS
                .iter()
                .map(|&t| Allpass::new(scale(t) + spread))
                .collect(),
        }
    }

    /// Process one mono sample; returns the decorrelated `(left, right)` tail.
    #[inline]
    pub(crate) fn process(&mut self, x: f32) -> (f32, f32) {
        let inp = x * INPUT_GAIN;
        let mut l = 0.0;
        for c in &mut self.comb_l {
            l += c.process(inp);
        }
        let mut r = 0.0;
        for c in &mut self.comb_r {
            r += c.process(inp);
        }
        for a in &mut self.ap_l {
            l = a.process(l);
        }
        for a in &mut self.ap_r {
            r = a.process(r);
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_produces_a_decaying_decorrelated_tail() {
        let mut rv = RoomReverb::new(48_000.0);
        let mut tail = 0.0;
        let mut decorr = 0.0;
        // Feed one impulse, then run silence and watch the tail.
        let _ = rv.process(1.0);
        for _ in 0..20_000 {
            let (l, r) = rv.process(0.0);
            tail += l.abs() + r.abs();
            decorr += (l - r).abs();
        }
        assert!(tail > 0.1, "expected a reverb tail, got {tail}");
        assert!(decorr > 0.0, "left/right should be decorrelated");
    }

    #[test]
    fn sustained_input_stays_bounded() {
        let mut rv = RoomReverb::new(48_000.0);
        let mut peak = 0.0_f32;
        for i in 0..96_000 {
            let x = if (i / 50) % 2 == 0 { 0.9 } else { -0.9 };
            let (l, r) = rv.process(x);
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak < 4.0, "reverb output should stay bounded, peak={peak}");
    }
}
