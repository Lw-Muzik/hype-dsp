//! 3D Surround: a ring of virtual loudspeakers rendered binaurally for
//! headphones (the category of Dolby Headphone / DTS Headphone:X / Windows
//! Sonic). Distinct from the lightweight [`crate::Spatializer`] crossfeed.
//!
//! Pipeline per stereo frame:
//! 1. **Upmix** the stereo pair into virtual-speaker feeds with a Pro Logic
//!    II–style passive matrix: fronts carry L/R, the rear surrounds carry the
//!    out-of-phase difference `(L−R)` (pre-delayed for envelopment), and the LFE
//!    is a low-passed mono sum.
//! 2. **Binaural render** each enabled speaker at its azimuth: the near
//!    (ipsilateral) ear hears the feed directly; the far (contralateral) ear
//!    hears it delayed by the inter-aural time difference, attenuated (ILD), and
//!    head-shadow low-passed — the cues the brain uses to localise azimuth.
//! 3. **Sum** all ears, add the LFE, normalise, and cross-fade against the dry
//!    signal by `intensity`.

use crate::delay::DelayLine;
use crate::reverb::RoomReverb;
use crate::{AudioProcessor, ProcessorParams};
use hm_core::SurroundSpeakers;

/// Effective head radius (m) for the spherical-head ITD model.
const HEAD_RADIUS_M: f32 = 0.0875;
/// Speed of sound (m/s).
const SOUND_SPEED: f32 = 343.0;
/// Virtual-speaker azimuths (degrees from front centre).
const FRONT_DEG: f32 = 30.0;
const SIDE_DEG: f32 = 90.0;
const SURROUND_DEG: f32 = 135.0;
/// Haas pre-delay on the rear feed so the surround image sits behind the head.
const SURROUND_PREDELAY_S: f32 = 0.006;
/// LFE / subwoofer crossover.
const LFE_HZ: f32 = 120.0;
/// Front↔tweeter crossover: content above this goes to the side "tweeters",
/// below it to the front speakers (a 2-way split, like a real driver crossover).
const TWEETER_HZ: f32 = 2000.0;
/// "8D" rotation rate of the rear/reverb field (Hz). Gentle ≈ one slow orbit
/// every ten seconds.
const ROTATION_HZ: f32 = 0.1;
/// Rotation depth: how far the rear balance swings (0 = none, 1 = full L↔R).
const ROTATION_DEPTH: f32 = 0.45;
/// Level of the reverberant rear (surround) send into the wet mix.
const SURROUND_LEVEL: f32 = 0.85;

/// One virtual loudspeaker: a near-ear direct path plus a far-ear path that is
/// delayed (ITD), attenuated (ILD), and head-shadow low-passed.
struct VirtualSpeaker {
    /// `true` if the near (full-level) ear is the left one.
    ipsi_left: bool,
    enabled: bool,
    feed_gain: f32,
    delay: DelayLine,
    delay_samples: usize,
    shadow_state: f32,
    shadow_coeff: f32,
    contra_gain: f32,
}

impl VirtualSpeaker {
    /// Build a speaker at `azimuth_deg` (sign picks the near ear) with a base
    /// `feed_gain`, computing its ITD / ILD / head-shadow for `sample_rate`.
    fn new(sample_rate: f32, azimuth_deg: f32, feed_gain: f32) -> Self {
        let theta = azimuth_deg.abs().to_radians();
        // Woodworth spherical-head ITD: t = (a/c)·(θ + sin θ).
        let itd = (HEAD_RADIUS_M / SOUND_SPEED) * (theta + theta.sin());
        let delay_samples = (itd * sample_rate).round().max(1.0) as usize;
        // Contralateral attenuation (ILD) and head-shadow cutoff both deepen as
        // the speaker swings to the side/rear.
        let front_bias = (1.0 + theta.cos()) * 0.5; // 1 at front … 0 behind
        let contra_gain = 0.30 + 0.70 * front_bias;
        let shadow_hz = 700.0 + 1300.0 * front_bias;
        let shadow_coeff = (-2.0 * std::f32::consts::PI * shadow_hz / sample_rate).exp();
        Self {
            ipsi_left: azimuth_deg < 0.0,
            enabled: true,
            feed_gain,
            delay: DelayLine::new(delay_samples),
            delay_samples,
            shadow_state: 0.0,
            shadow_coeff,
            contra_gain,
        }
    }

    /// Per-ear contribution `(left, right)` for feed sample `s`.
    #[inline]
    fn render(&mut self, s: f32) -> (f32, f32) {
        if !self.enabled {
            return (0.0, 0.0);
        }
        let near = s * self.feed_gain;
        let delayed = self.delay.process(near, self.delay_samples);
        self.shadow_state =
            self.shadow_state * self.shadow_coeff + delayed * (1.0 - self.shadow_coeff);
        let far = self.shadow_state * self.contra_gain;
        if self.ipsi_left {
            (near, far)
        } else {
            (far, near)
        }
    }

    /// Gain this speaker contributes to the *left* ear (near level if it sits on
    /// the left, far/ILD level otherwise). Summed over speakers and inverted to
    /// normalise the field toward per-ear unity. By symmetry the right ear sums
    /// to the same value.
    fn left_ear_gain(&self) -> f32 {
        if self.ipsi_left {
            self.feed_gain
        } else {
            self.feed_gain * self.contra_gain
        }
    }
}

/// The 3D-surround stage: six virtual speakers, an LFE path, and a dry/wet mix.
pub struct Surround3D {
    sample_rate: f32,
    channels: usize,
    enabled: bool,
    intensity: f32,
    subwoofer: f32,
    speakers_state: SurroundSpeakers,
    front_l: VirtualSpeaker,
    front_r: VirtualSpeaker,
    side_l: VirtualSpeaker,
    side_r: VirtualSpeaker,
    surround_l: VirtualSpeaker,
    surround_r: VirtualSpeaker,
    rear_predelay: DelayLine,
    rear_predelay_samples: usize,
    lfe_state: f32,
    lfe_coeff: f32,
    /// One-pole low-pass state per channel for the front↔tweeter crossover.
    /// `lp` feeds the front; `input − lp` (the highs) feeds the tweeter.
    xover_lp: [f32; 2],
    xover_coeff: f32,
    /// Diffuse room reverb feeding the surround (rear) speakers.
    reverb: RoomReverb,
    /// "8D" rotation oscillator phase and per-sample increment.
    rot_phase: f32,
    rot_inc: f32,
    /// `true` when at least one rear speaker is on (the reverb send is live).
    surround_on: bool,
    /// 1 / (sum of enabled front+side per-ear gains): keeps the direct field
    /// near unity. The reverberant rear send is mixed separately at
    /// [`SURROUND_LEVEL`], so it isn't normalised away.
    wet_norm: f32,
}

impl Surround3D {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut s = Self {
            sample_rate,
            channels: channels.max(1),
            enabled: false,
            intensity: 0.0,
            subwoofer: 0.0,
            speakers_state: SurroundSpeakers::default(),
            front_l: VirtualSpeaker::new(sample_rate, -FRONT_DEG, 1.0),
            front_r: VirtualSpeaker::new(sample_rate, FRONT_DEG, 1.0),
            side_l: VirtualSpeaker::new(sample_rate, -SIDE_DEG, 0.6),
            side_r: VirtualSpeaker::new(sample_rate, SIDE_DEG, 0.6),
            surround_l: VirtualSpeaker::new(sample_rate, -SURROUND_DEG, 0.55),
            surround_r: VirtualSpeaker::new(sample_rate, SURROUND_DEG, 0.55),
            rear_predelay: DelayLine::new(1),
            rear_predelay_samples: 0,
            lfe_state: 0.0,
            lfe_coeff: 0.0,
            xover_lp: [0.0; 2],
            xover_coeff: 0.0,
            reverb: RoomReverb::new(sample_rate),
            rot_phase: 0.0,
            rot_inc: 0.0,
            surround_on: true,
            wet_norm: 1.0,
        };
        s.reconfigure();
        s
    }

    fn reconfigure(&mut self) {
        self.front_l = VirtualSpeaker::new(self.sample_rate, -FRONT_DEG, 1.0);
        self.front_r = VirtualSpeaker::new(self.sample_rate, FRONT_DEG, 1.0);
        self.side_l = VirtualSpeaker::new(self.sample_rate, -SIDE_DEG, 0.6);
        self.side_r = VirtualSpeaker::new(self.sample_rate, SIDE_DEG, 0.6);
        self.surround_l = VirtualSpeaker::new(self.sample_rate, -SURROUND_DEG, 0.55);
        self.surround_r = VirtualSpeaker::new(self.sample_rate, SURROUND_DEG, 0.55);
        self.rear_predelay_samples =
            (SURROUND_PREDELAY_S * self.sample_rate).round().max(1.0) as usize;
        self.rear_predelay = DelayLine::new(self.rear_predelay_samples);
        self.lfe_state = 0.0;
        self.lfe_coeff = (-2.0 * std::f32::consts::PI * LFE_HZ / self.sample_rate).exp();
        self.xover_lp = [0.0; 2];
        self.xover_coeff = (-2.0 * std::f32::consts::PI * TWEETER_HZ / self.sample_rate).exp();
        self.reverb = RoomReverb::new(self.sample_rate);
        self.rot_phase = 0.0;
        self.rot_inc = 2.0 * std::f32::consts::PI * ROTATION_HZ / self.sample_rate;
        self.apply_speaker_states();
    }

    /// Push the on/off flags into the speakers and recompute `wet_norm`.
    fn apply_speaker_states(&mut self) {
        let sp = self.speakers_state;
        self.front_l.enabled = sp.front_l;
        self.front_r.enabled = sp.front_r;
        self.side_l.enabled = sp.side_l;
        self.side_r.enabled = sp.side_r;
        self.surround_l.enabled = sp.surround_l;
        self.surround_r.enabled = sp.surround_r;
        self.surround_on = sp.surround_l || sp.surround_r;

        // Normalise the *direct* field (front + side) toward per-ear unity. The
        // reverberant rear send is mixed separately (SURROUND_LEVEL), so it is
        // intentionally excluded here.
        let ear_gain = [&self.front_l, &self.front_r, &self.side_l, &self.side_r]
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| s.left_ear_gain())
            .sum::<f32>();
        self.wet_norm = if ear_gain > 1e-6 { 1.0 / ear_gain } else { 0.0 };
    }

    /// Whether the stage would change the signal (used as a fast bypass).
    fn is_active(&self) -> bool {
        self.enabled
            && self.intensity > 0.0
            && (self.wet_norm > 0.0 || self.subwoofer > 0.0 || self.surround_on)
    }
}

impl AudioProcessor for Surround3D {
    fn prepare(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.max(1);
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if channels < 2 || !self.is_active() {
            return;
        }
        let intensity = self.intensity;
        let dry = 1.0 - intensity;
        let sub = self.subwoofer;
        let norm = self.wet_norm;
        let lfe_a = self.lfe_coeff;
        let cx = self.xover_coeff;
        let rear_d = self.rear_predelay_samples;
        let side_l_on = self.side_l.enabled;
        let side_r_on = self.side_r.enabled;
        let surround_on = self.surround_on;
        let sl_on = self.surround_l.enabled;
        let sr_on = self.surround_r.enabled;
        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let l = buffer[base];
            let r = buffer[base + 1];

            // 2-way crossover per channel: lows/mids → front, highs → tweeter
            // (one-pole complementary split: lp + hp == input). When a tweeter
            // is off, its highs fall back to the front so treble isn't lost.
            self.xover_lp[0] = self.xover_lp[0] * cx + l * (1.0 - cx);
            self.xover_lp[1] = self.xover_lp[1] * cx + r * (1.0 - cx);
            let lo_l = self.xover_lp[0];
            let lo_r = self.xover_lp[1];
            let hi_l = l - lo_l;
            let hi_r = r - lo_r;
            let front_l_in = lo_l + if side_l_on { 0.0 } else { hi_l };
            let front_r_in = lo_r + if side_r_on { 0.0 } else { hi_r };

            // --- Direct field: front + side, binaurally summed and normalised.
            let mut el = 0.0;
            let mut er = 0.0;
            let (a, b) = self.front_l.render(front_l_in);
            el += a;
            er += b;
            let (a, b) = self.front_r.render(front_r_in);
            el += a;
            er += b;
            let (a, b) = self.side_l.render(hi_l);
            el += a;
            er += b;
            let (a, b) = self.side_r.render(hi_r);
            el += a;
            er += b;

            let mut wl = el * norm;
            let mut wr = er * norm;

            // --- Reverberant rear field: a diffuse room reverb fed by the mono
            // program, slowly rotated ("8D") and positioned at the rear ±135°.
            if surround_on {
                let mono = self.rear_predelay.process((l + r) * 0.5, rear_d);
                let (rv_l, rv_r) = self.reverb.process(mono);
                let lfo = self.rot_phase.sin();
                self.rot_phase += self.rot_inc;
                if self.rot_phase >= std::f32::consts::TAU {
                    self.rot_phase -= std::f32::consts::TAU;
                }
                let rot_l = 1.0 + ROTATION_DEPTH * lfo;
                let rot_r = 1.0 - ROTATION_DEPTH * lfo;
                let rear_l = if sl_on { rv_l * rot_l } else { 0.0 };
                let rear_r = if sr_on { rv_r * rot_r } else { 0.0 };
                let (a, b) = self.surround_l.render(rear_l);
                wl += SURROUND_LEVEL * a;
                wr += SURROUND_LEVEL * b;
                let (a, b) = self.surround_r.render(rear_r);
                wl += SURROUND_LEVEL * a;
                wr += SURROUND_LEVEL * b;
            }

            // LFE: low-passed mono added equally to both ears.
            if sub > 0.0 {
                let mid = (l + r) * 0.5;
                self.lfe_state = self.lfe_state * lfe_a + mid * (1.0 - lfe_a);
                let lfe = self.lfe_state * sub;
                wl += lfe;
                wr += lfe;
            }

            buffer[base] = l * dry + wl * intensity;
            buffer[base + 1] = r * dry + wr * intensity;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        let s = &params.surround3d;
        self.enabled = s.enabled;
        self.intensity = s.intensity.clamp(0.0, 1.0);
        self.subwoofer = s.subwoofer.clamp(0.0, 1.0);
        if self.speakers_state != s.speakers {
            self.speakers_state = s.speakers;
            self.apply_speaker_states();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, Surround3DState, SurroundSpeakers};

    fn stereo(pairs: &[(f32, f32)]) -> Vec<f32> {
        pairs.iter().flat_map(|&(l, r)| [l, r]).collect()
    }

    fn state(intensity: f32, subwoofer: f32, speakers: SurroundSpeakers) -> EngineState {
        EngineState {
            surround3d: Surround3DState {
                enabled: true,
                intensity,
                subwoofer,
                speakers,
            },
            ..Default::default()
        }
    }

    /// Enable only the named pairs (surrounds off) to isolate a band/position.
    fn speakers_only(side: bool, front: bool) -> SurroundSpeakers {
        SurroundSpeakers {
            front_l: front,
            front_r: front,
            side_l: side,
            side_r: side,
            surround_l: false,
            surround_r: false,
        }
    }

    fn energy(buf: &[f32]) -> f32 {
        buf.iter().map(|x| x * x).sum()
    }

    fn sine_stereo(freq: f32, frames: usize) -> Vec<f32> {
        let sr = 48_000.0;
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / sr;
                let s = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn disabled_is_identity() {
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&EngineState::default()); // disabled
        let input = stereo(&[(0.5, -0.3), (0.2, 0.4)]);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn intensity_zero_is_identity() {
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(0.0, 0.5, SurroundSpeakers::default()));
        let input = stereo(&[(0.5, -0.3), (0.2, 0.4), (0.1, 0.1)]);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);
        assert_eq!(buf, input);
    }

    #[test]
    fn enabled_changes_signal_but_stays_bounded() {
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.5, SurroundSpeakers::default()));
        let input = sine_stereo(440.0, 256);
        let mut buf = input.clone();
        sp.process(&mut buf, 2);
        assert!(buf != input, "surround should transform the signal");
        assert!(buf.iter().all(|&x| x.abs() < 2.0), "output blew up");
    }

    #[test]
    fn disabling_a_speaker_changes_output() {
        // Long enough for the rear reverb (pre-delay + tail) to engage.
        let input = sine_stereo(440.0, 8_192);

        let mut all = Surround3D::new(48_000.0, 2);
        all.set_params(&state(1.0, 0.0, SurroundSpeakers::default()));
        let mut a = input.clone();
        all.process(&mut a, 2);

        let mut without_rear = Surround3D::new(48_000.0, 2);
        without_rear.set_params(&state(
            1.0,
            0.0,
            SurroundSpeakers {
                surround_l: false,
                surround_r: false,
                ..SurroundSpeakers::default()
            },
        ));
        let mut b = input.clone();
        without_rear.process(&mut b, 2);

        assert!(a != b, "toggling the rear speakers must change the output");
    }

    #[test]
    fn subwoofer_level_adds_low_frequency_energy() {
        let input = sine_stereo(60.0, 1024); // below the 120 Hz LFE crossover

        let mut dry_sub = Surround3D::new(48_000.0, 2);
        dry_sub.set_params(&state(1.0, 0.0, SurroundSpeakers::default()));
        let mut no_sub = input.clone();
        dry_sub.process(&mut no_sub, 2);

        let mut wet_sub = Surround3D::new(48_000.0, 2);
        wet_sub.set_params(&state(1.0, 1.0, SurroundSpeakers::default()));
        let mut full_sub = input.clone();
        wet_sub.process(&mut full_sub, 2);

        assert!(
            energy(&full_sub) > energy(&no_sub),
            "more subwoofer should add low-frequency energy: {} !> {}",
            energy(&full_sub),
            energy(&no_sub)
        );
    }

    #[test]
    fn intensity_scales_the_effect() {
        let input = sine_stereo(440.0, 256);
        let dry = input.clone();

        let mut half = Surround3D::new(48_000.0, 2);
        half.set_params(&state(0.5, 0.0, SurroundSpeakers::default()));
        let mut h = input.clone();
        half.process(&mut h, 2);

        let mut full = Surround3D::new(48_000.0, 2);
        full.set_params(&state(1.0, 0.0, SurroundSpeakers::default()));
        let mut f = input.clone();
        full.process(&mut f, 2);

        let dev = |buf: &[f32]| -> f32 {
            buf.iter().zip(&dry).map(|(a, b)| (a - b).abs()).sum()
        };
        assert!(
            dev(&f) > dev(&h),
            "higher intensity should diverge further from dry: {} !> {}",
            dev(&f),
            dev(&h)
        );
    }

    #[test]
    fn tweeters_carry_highs_not_lows() {
        // The side "Tweeter" speakers are high-frequency drivers: they must pass
        // treble and reject bass, not merely steer a full-range signal.
        let only_tweeters = speakers_only(true, false);

        let mut lo = Surround3D::new(48_000.0, 2);
        lo.set_params(&state(1.0, 0.0, only_tweeters));
        let mut low = sine_stereo(300.0, 2048);
        lo.process(&mut low, 2);

        let mut hi = Surround3D::new(48_000.0, 2);
        hi.set_params(&state(1.0, 0.0, only_tweeters));
        let mut high = sine_stereo(9000.0, 2048);
        hi.process(&mut high, 2);

        assert!(
            energy(&high) > 5.0 * energy(&low),
            "tweeters should pass highs, not lows: high={} low={}",
            energy(&high),
            energy(&low)
        );
    }

    #[test]
    fn disabling_tweeters_keeps_treble_on_front() {
        // With the tweeters off, their high-frequency content must fall back to
        // the front speakers — turning a tweeter off shouldn't mute the treble.
        let only_fronts = speakers_only(false, true);
        let input = sine_stereo(9000.0, 2048);
        let dry = energy(&input);

        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.0, only_fronts));
        let mut out = input.clone();
        sp.process(&mut out, 2);

        // Guard: with the crossover, omitting the fallback would route highs
        // only to the (disabled) tweeters and drop them entirely (~0). The
        // fallback keeps the front full-range, so treble stays well above zero.
        assert!(
            energy(&out) > 0.05 * dry,
            "treble must survive with tweeters off (front full-range fallback): {} vs dry {}",
            energy(&out),
            dry
        );
    }

    fn surround_only() -> SurroundSpeakers {
        SurroundSpeakers {
            front_l: false,
            front_r: false,
            side_l: false,
            side_r: false,
            surround_l: true,
            surround_r: true,
        }
    }

    #[test]
    fn surround_has_a_reverb_tail() {
        // A large room keeps ringing after the input stops.
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.0, surround_only()));
        let burst = 4_800; // 0.1 s
        let silence = 14_400; // 0.3 s
        let mut buf = sine_stereo(440.0, burst);
        buf.extend(std::iter::repeat_n(0.0, silence * 2));
        sp.process(&mut buf, 2);
        let tail = &buf[(burst + 2_400) * 2..]; // 50 ms after the burst ends
        assert!(
            energy(tail) > 1e-3,
            "expected a decaying reverb tail, got {}",
            energy(tail)
        );
    }

    #[test]
    fn surround_fills_the_room_for_mono_and_decorrelates() {
        // Mono content must still envelop the listener (evenly distributed),
        // with the two surround channels decorrelated — the old static-difference
        // design produced silence for mono.
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.0, surround_only()));
        let mut buf = sine_stereo(440.0, 9_600); // mono (L == R)
        sp.process(&mut buf, 2);
        let region = &buf[4_800 * 2..];
        let level: f32 = region.iter().map(|x| x.abs()).sum();
        let diff: f32 = region.chunks(2).map(|c| (c[0] - c[1]).abs()).sum();
        assert!(level > 1e-2, "surround should fill the room for mono input");
        assert!(
            diff > 0.1 * level,
            "surround channels should be decorrelated: diff={diff} level={level}"
        );
    }

    #[test]
    fn surround_field_rotates_over_time() {
        // The "8D" motion: the rear balance drifts around the head over time.
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.0, surround_only()));
        let frames = 48_000 * 4; // 4 s
        let mut buf = sine_stereo(440.0, frames);
        sp.process(&mut buf, 2);
        let balance = |s: &[f32]| -> f32 {
            let l: f32 = s.chunks(2).map(|c| c[0].abs()).sum();
            let r: f32 = s.chunks(2).map(|c| c[1].abs()).sum();
            (l - r) / (l + r + 1e-9)
        };
        let early = balance(&buf[20_000 * 2..40_000 * 2]);
        let late = balance(&buf[(frames - 40_000) * 2..(frames - 20_000) * 2]);
        assert!(
            (early - late).abs() > 0.05,
            "rear field should rotate (balance shift): early={early} late={late}"
        );
    }

    #[test]
    fn surround_reverb_stays_bounded() {
        // Stability: a feedback reverb must never blow up, even on sustained,
        // hard-panned, full-scale input.
        let mut sp = Surround3D::new(48_000.0, 2);
        sp.set_params(&state(1.0, 0.5, SurroundSpeakers::default()));
        let frames = 48_000 * 2;
        let mut buf = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = if (i / 64) % 2 == 0 { 0.9 } else { -0.9 };
            buf.push(s);
            buf.push(-s);
        }
        sp.process(&mut buf, 2);
        assert!(
            buf.iter().all(|&x| x.abs() < 2.0),
            "reverb must stay bounded"
        );
    }
}
