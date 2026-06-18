//! Room reverb ("room effects") — a Freeverb-style algorithmic reverb ported
//! from the Hype mobile app: 8 parallel lowpass-feedback comb filters into
//! 4 series Schroeder allpass diffusers, per channel, with pre-delay and a
//! wet/dry mix.
//!
//! Params ([`hm_core::RoomState`]): **room size** (scales the comb delays),
//! **decay** (tail length → comb feedback), **damping** (HF absorption),
//! **pre-delay** (ms), **diffusion** (allpass feedback / echo density), and
//! **wet/dry**. Unlike the mobile build — where room size was computed but never
//! applied — here room size actually scales the comb delay lengths.
//!
//! Reference: Jezar's Freeverb (<https://ccrma.stanford.edu/~jos/pasp/Freeverb.html>).

use crate::{AudioProcessor, ProcessorParams};

/// Jezar's comb tunings @44.1 kHz; the right channel is offset for decorrelation.
const COMB_TUNING_L: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const COMB_TUNING_R: [usize; 8] = [1139, 1211, 1300, 1379, 1445, 1514, 1580, 1640];
const ALLPASS_TUNING_L: [usize; 4] = [556, 441, 341, 225];
const ALLPASS_TUNING_R: [usize; 4] = [579, 464, 364, 248];
const REFERENCE_SR: f32 = 44_100.0;
const FIXED_GAIN: f32 = 0.015;
const SCALE_ROOM: f32 = 0.28;
const OFFSET_ROOM: f32 = 0.7;
const SCALE_DAMP: f32 = 0.4;
const MAX_PREDELAY_MS: f32 = 200.0;

/// Flush near-denormal values to zero so the IIR feedback tails don't decay into
/// the denormal range (which can cause large CPU slowdowns).
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 {
        0.0
    } else {
        x
    }
}

/// Lowpass-feedback comb filter with a settable active delay (≤ capacity), so
/// room size can scale the delay at runtime without reallocating.
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    delay: usize,
    store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl Comb {
    fn new(cap: usize, delay: usize) -> Self {
        let cap = cap.max(1);
        Self {
            buf: vec![0.0; cap],
            idx: 0,
            delay: delay.clamp(1, cap),
            store: 0.0,
            feedback: 0.84,
            damp1: 0.2,
            damp2: 0.8,
        }
    }
    fn set_delay(&mut self, d: usize) {
        self.delay = d.clamp(1, self.buf.len());
    }
    fn set_feedback(&mut self, f: f32) {
        self.feedback = f;
    }
    fn set_damp(&mut self, d: f32) {
        self.damp1 = d;
        self.damp2 = 1.0 - d;
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let n = self.buf.len();
        let read = (self.idx + n - self.delay) % n;
        let out = self.buf[read];
        self.store = flush(out * self.damp2 + self.store * self.damp1);
        self.buf[self.idx] = x + self.store * self.feedback;
        self.idx = if self.idx + 1 == n { 0 } else { self.idx + 1 };
        out
    }
}

/// Schroeder allpass diffuser (energy-preserving: |H| = 1).
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
    feedback: f32,
}

impl Allpass {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0.0; size.max(1)],
            idx: 0,
            feedback: 0.5,
        }
    }
    fn set_feedback(&mut self, f: f32) {
        self.feedback = f;
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let n = self.buf.len();
        let bufout = self.buf[self.idx];
        let out = -x + bufout;
        self.buf[self.idx] = flush(x + bufout * self.feedback);
        self.idx = if self.idx + 1 == n { 0 } else { self.idx + 1 };
        out
    }
}

/// Fixed pre-delay line.
struct Delay {
    buf: Vec<f32>,
    idx: usize,
}

impl Delay {
    fn new(cap: usize) -> Self {
        Self {
            buf: vec![0.0; cap.max(1)],
            idx: 0,
        }
    }
    #[inline]
    fn process(&mut self, x: f32, delay: usize) -> f32 {
        let n = self.buf.len();
        if delay == 0 {
            return x;
        }
        let d = delay.min(n - 1);
        let read = (self.idx + n - d) % n;
        let out = self.buf[read];
        self.buf[self.idx] = x;
        self.idx = if self.idx + 1 == n { 0 } else { self.idx + 1 };
        out
    }
}

/// The room reverb processing stage.
pub struct RoomEffects {
    sample_rate: f32,
    enabled: bool,
    wet: f32,
    // Cached params (change-guarded so re-tuning is cheap when unchanged).
    room_size: f32,
    decay: f32,
    damping: f32,
    diffusion: f32,
    pre_delay_ms: f32,
    pre_delay_samples: usize,
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpass_l: Vec<Allpass>,
    allpass_r: Vec<Allpass>,
    predelay_l: Delay,
    predelay_r: Delay,
}

impl RoomEffects {
    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let mut s = Self {
            sample_rate,
            enabled: false,
            wet: 0.0,
            room_size: 0.4,
            decay: 0.4,
            damping: 0.45,
            diffusion: 0.55,
            pre_delay_ms: 8.0,
            pre_delay_samples: 0,
            combs_l: Vec::new(),
            combs_r: Vec::new(),
            allpass_l: Vec::new(),
            allpass_r: Vec::new(),
            predelay_l: Delay::new(1),
            predelay_r: Delay::new(1),
        };
        s.reconfigure();
        s
    }

    /// (Re)allocate the filter buffers for the current sample rate.
    fn reconfigure(&mut self) {
        let scale = self.sample_rate / REFERENCE_SR;
        let comb = |tuning: &[usize; 8]| -> Vec<Comb> {
            tuning
                .iter()
                .map(|&t| {
                    let cap = (t as f32 * scale * 2.0) as usize + 16;
                    Comb::new(cap, cap / 2)
                })
                .collect()
        };
        let allpass = |tuning: &[usize; 4]| -> Vec<Allpass> {
            tuning
                .iter()
                .map(|&t| Allpass::new(((t as f32 * scale) as usize).max(1)))
                .collect()
        };
        self.combs_l = comb(&COMB_TUNING_L);
        self.combs_r = comb(&COMB_TUNING_R);
        self.allpass_l = allpass(&ALLPASS_TUNING_L);
        self.allpass_r = allpass(&ALLPASS_TUNING_R);
        let pd_cap = (self.sample_rate * 0.25) as usize + 16;
        self.predelay_l = Delay::new(pd_cap);
        self.predelay_r = Delay::new(pd_cap);
        self.retune();
    }

    /// Recompute derived coefficients from the cached params.
    fn retune(&mut self) {
        let scale = self.sample_rate / REFERENCE_SR;
        let size_scale = 0.5 + 1.5 * self.room_size;
        let feedback = (self.decay * SCALE_ROOM + OFFSET_ROOM).clamp(0.0, 0.98);
        let damp = self.damping * SCALE_DAMP;
        let ap_fb = 0.3 + self.diffusion * 0.4;
        for (i, c) in self.combs_l.iter_mut().enumerate() {
            c.set_delay(((COMB_TUNING_L[i] as f32 * scale * size_scale) as usize).max(1));
            c.set_feedback(feedback);
            c.set_damp(damp);
        }
        for (i, c) in self.combs_r.iter_mut().enumerate() {
            c.set_delay(((COMB_TUNING_R[i] as f32 * scale * size_scale) as usize).max(1));
            c.set_feedback(feedback);
            c.set_damp(damp);
        }
        for a in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) {
            a.set_feedback(ap_fb);
        }
        self.pre_delay_samples = (self.pre_delay_ms * 0.001 * self.sample_rate) as usize;
    }
}

impl AudioProcessor for RoomEffects {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        self.sample_rate = sample_rate;
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || self.wet <= 0.0 || channels == 0 {
            return;
        }
        let wet = self.wet;
        let dry = 1.0 - wet;
        let pds = self.pre_delay_samples;
        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let (l, r) = if channels >= 2 {
                (buffer[base], buffer[base + 1])
            } else {
                (buffer[base], buffer[base])
            };
            let pdl = self.predelay_l.process(l, pds);
            let pdr = self.predelay_r.process(r, pds);
            let input = (pdl + pdr) * FIXED_GAIN;

            let mut out_l = 0.0;
            let mut out_r = 0.0;
            for c in self.combs_l.iter_mut() {
                out_l += c.process(input);
            }
            for c in self.combs_r.iter_mut() {
                out_r += c.process(input);
            }
            for a in self.allpass_l.iter_mut() {
                out_l = a.process(out_l);
            }
            for a in self.allpass_r.iter_mut() {
                out_r = a.process(out_r);
            }

            let mix_l = (l * dry + out_l * wet).clamp(-4.0, 4.0);
            let mix_r = (r * dry + out_r * wet).clamp(-4.0, 4.0);
            if channels >= 2 {
                buffer[base] = mix_l;
                buffer[base + 1] = mix_r;
            } else {
                buffer[base] = (mix_l + mix_r) * 0.5;
            }
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        let r = &params.room;
        self.enabled = r.enabled;
        self.wet = r.wet_dry.clamp(0.0, 1.0);
        let changed = (self.room_size - r.room_size).abs() > f32::EPSILON
            || (self.decay - r.decay).abs() > f32::EPSILON
            || (self.damping - r.damping).abs() > f32::EPSILON
            || (self.diffusion - r.diffusion).abs() > f32::EPSILON
            || (self.pre_delay_ms - r.pre_delay).abs() > f32::EPSILON;
        if changed {
            self.room_size = r.room_size.clamp(0.0, 1.0);
            self.decay = r.decay.clamp(0.0, 1.0);
            self.damping = r.damping.clamp(0.0, 1.0);
            self.diffusion = r.diffusion.clamp(0.0, 1.0);
            self.pre_delay_ms = r.pre_delay.clamp(0.0, MAX_PREDELAY_MS);
            self.retune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, RoomState};

    fn stereo(pairs: &[(f32, f32)]) -> Vec<f32> {
        pairs.iter().flat_map(|&(l, r)| [l, r]).collect()
    }

    fn sine(freq: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = (2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    fn energy(b: &[f32]) -> f32 {
        b.iter().map(|x| x * x).sum()
    }

    fn state(enabled: bool, wet: f32, decay: f32, room_size: f32) -> EngineState {
        EngineState {
            room: RoomState {
                enabled,
                room_size,
                decay,
                damping: 0.3,
                pre_delay: 5.0,
                diffusion: 0.5,
                wet_dry: wet,
                active_preset_id: None,
            },
            ..Default::default()
        }
    }

    #[test]
    fn disabled_is_identity() {
        let mut rv = RoomEffects::new(48_000.0, 2);
        rv.set_params(&EngineState::default()); // off
        let input = stereo(&[(0.5, -0.3), (0.2, 0.4)]);
        let mut buf = input.clone();
        rv.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn fully_dry_is_identity() {
        let mut rv = RoomEffects::new(48_000.0, 2);
        rv.set_params(&state(true, 0.0, 0.5, 0.5)); // enabled but wet=0
        let input = stereo(&[(0.5, -0.3), (0.2, 0.4)]);
        let mut buf = input.clone();
        rv.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn produces_a_decaying_tail() {
        let mut rv = RoomEffects::new(48_000.0, 2);
        rv.set_params(&state(true, 0.5, 0.6, 0.5));
        // 0.1s burst, then silence.
        let mut buf = sine(440.0, 4_800);
        buf.extend(std::iter::repeat_n(0.0, 24_000 * 2));
        rv.process(&mut buf, 2);
        let tail = &buf[(4_800 + 4_800) * 2..]; // well after the burst
        assert!(energy(tail) > 1e-3, "expected a reverb tail, got {}", energy(tail));
    }

    #[test]
    fn fills_both_channels_decorrelated_for_mono() {
        let mut rv = RoomEffects::new(48_000.0, 2);
        rv.set_params(&state(true, 0.6, 0.6, 0.6));
        let mut buf = sine(440.0, 9_600); // mono (L == R)
        rv.process(&mut buf, 2);
        let region = &buf[4_800 * 2..];
        let diff: f32 = region.chunks(2).map(|c| (c[0] - c[1]).abs()).sum();
        assert!(diff > 0.1, "reverb channels should decorrelate: {diff}");
    }

    #[test]
    fn longer_decay_gives_a_longer_tail() {
        let burst = 4_800;
        let make = |decay: f32| {
            let mut rv = RoomEffects::new(48_000.0, 2);
            rv.set_params(&state(true, 1.0, decay, 0.5));
            let mut buf = sine(440.0, burst);
            buf.extend(std::iter::repeat_n(0.0, 48_000 * 2));
            rv.process(&mut buf, 2);
            energy(&buf[(burst + 24_000) * 2..]) // late tail energy
        };
        let short = make(0.1);
        let long = make(0.9);
        assert!(long > short * 2.0, "more decay → longer tail: long={long} short={short}");
    }

    #[test]
    fn stays_bounded_under_sustained_input() {
        let mut rv = RoomEffects::new(48_000.0, 2);
        rv.set_params(&state(true, 1.0, 0.95, 1.0));
        let mut buf = stereo(&[(0.9, -0.9), (-0.8, 0.8), (0.95, 0.95)].repeat(4_000));
        rv.process(&mut buf, 2);
        assert!(buf.iter().all(|&x| x.abs() <= 4.0), "reverb must stay bounded");
    }
}
