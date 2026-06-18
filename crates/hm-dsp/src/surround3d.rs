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
    /// 1 / (sum of enabled per-ear gains): keeps the summed field near unity.
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

        // Per-ear gain summed over enabled speakers; its reciprocal normalises
        // the wet field so the effect doesn't change perceived loudness.
        let ear_gain = self
            .all_speakers()
            .filter(|s| s.enabled)
            .map(|s| s.left_ear_gain())
            .sum::<f32>();
        self.wet_norm = if ear_gain > 1e-6 { 1.0 / ear_gain } else { 0.0 };
    }

    fn all_speakers(&self) -> impl Iterator<Item = &VirtualSpeaker> {
        [
            &self.front_l,
            &self.front_r,
            &self.side_l,
            &self.side_r,
            &self.surround_l,
            &self.surround_r,
        ]
        .into_iter()
    }

    /// Whether the stage would change the signal (used as a fast bypass).
    fn is_active(&self) -> bool {
        self.enabled && self.intensity > 0.0 && (self.wet_norm > 0.0 || self.subwoofer > 0.0)
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
        let rear_d = self.rear_predelay_samples;
        let frames = buffer.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let l = buffer[base];
            let r = buffer[base + 1];
            // Rear feed: the out-of-phase difference, Haas pre-delayed so the
            // surround image is perceived behind the listener.
            let rear = self.rear_predelay.process((l - r) * 0.5, rear_d);

            // Binaural-render each enabled speaker, summing per ear.
            let mut el = 0.0;
            let mut er = 0.0;
            let (a, b) = self.front_l.render(l);
            el += a;
            er += b;
            let (a, b) = self.front_r.render(r);
            el += a;
            er += b;
            let (a, b) = self.side_l.render(l);
            el += a;
            er += b;
            let (a, b) = self.side_r.render(r);
            el += a;
            er += b;
            let (a, b) = self.surround_l.render(rear);
            el += a;
            er += b;
            let (a, b) = self.surround_r.render(-rear);
            el += a;
            er += b;

            let mut wl = el * norm;
            let mut wr = er * norm;

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
        let input = sine_stereo(440.0, 256);

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
}
