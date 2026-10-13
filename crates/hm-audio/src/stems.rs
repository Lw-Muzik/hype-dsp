//! Live stem playback: **Vocals / Drums / Instruments**, mixed from a
//! progressively separated track.
//!
//! The mobile app's formula, kept exactly:
//!
//! ```text
//! out = gV·V + gD·D + gI·(M − V − D)
//! ```
//!
//! where `M` is the decoded mix and `V`, `D` the separated vocals and drums;
//! instruments are whatever is left (`I = M − V − D`: bass, keys, guitars, the
//! residual). It is evaluated as `M + (gV−1)·V + (gD−1)·D + (gI−1)·I`, which is
//! the same number but makes every gain at unity return `M` **bit for bit** —
//! no rounding from adding back what was subtracted.
//!
//! Separation fills in behind playback, so the source asks the shared
//! [`StemStore`] chunk by chunk: where a chunk isn't ready yet the original mix
//! plays untouched, and the gains glide back to the user's setting once it
//! is. The source also tells the store where the listener is, so the worker
//! separates what is about to be heard (and what the listener seeks to) first.
//!
//! Everything is stored at 44.1 kHz; the source resamples (linearly, exactly as
//! the engine does for a plain file) to the device rate on the fly.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use hm_stems::{StemStore, MODEL_RATE};

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// Number of live controls: vocals, drums, instruments.
pub const ELEMENT_COUNT: usize = 3;
pub const EL_VOCALS: usize = 0;
pub const EL_DRUMS: usize = 1;
pub const EL_INSTRUMENTS: usize = 2;

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
            let g = if gain.is_finite() { gain.clamp(0.0, 2.0) } else { 1.0 };
            self.gains[element].store(g.to_bits(), Ordering::Relaxed);
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

/// Per-frame smoothing toward the target gain (~8 ms at 48 kHz): toggling a
/// control, or a chunk becoming ready, doesn't click.
const GAIN_SMOOTH: f32 = 0.0025;

/// One mixed frame. `m`, `v`, `d` are the stereo mix, vocals and drums at the
/// same instant; `g` the (smoothed) gains.
#[inline]
pub fn mix_frame(m: [f32; 2], v: [f32; 2], d: [f32; 2], g: [f32; ELEMENT_COUNT]) -> [f32; 2] {
    let (kv, kd, ki) = (g[EL_VOCALS] - 1.0, g[EL_DRUMS] - 1.0, g[EL_INSTRUMENTS] - 1.0);
    std::array::from_fn(|c| {
        let i = m[c] - v[c] - d[c];
        m[c] + kv * v[c] + kd * d[c] + ki * i
    })
}

/// The output frame the stem mixer starts at when it replaces the file player
/// at `start_secs` (the live playhead): same instant, at the device rate, never
/// past the end.
pub fn swap_start_frame(start_secs: f64, device_rate: u32, total_frames: usize) -> usize {
    let f = (start_secs.max(0.0) * f64::from(device_rate)).round();
    (f as usize).min(total_frames)
}

/// Plays a [`StemStore`] through the engine at the device rate.
pub struct StemPlaybackSource {
    store: Arc<StemStore>,
    gains: Arc<StemGains>,
    smoothed: [f32; ELEMENT_COUNT],
    /// Model frames per output frame (`44100 / device_rate`).
    step: f64,
    cursor: usize,
    len_frames: usize,
}

impl StemPlaybackSource {
    pub fn new(store: Arc<StemStore>, gains: Arc<StemGains>, device_rate: u32) -> Self {
        let device_rate = device_rate.max(1);
        let len_frames =
            (store.frames() as f64 * f64::from(device_rate) / f64::from(MODEL_RATE)).round() as usize;
        let smoothed = gains.snapshot();
        Self {
            store,
            gains,
            smoothed,
            step: f64::from(MODEL_RATE) / f64::from(device_rate),
            cursor: 0,
            len_frames,
        }
    }

    /// The mixed stereo frame at model-domain index `idx`, and whether the
    /// separated stems exist there.
    #[inline]
    fn frame_at(&self, idx: usize, g: [f32; ELEMENT_COUNT]) -> ([f32; 2], bool) {
        let mix = self.store.mix();
        let m = [mix[idx * 2], mix[idx * 2 + 1]];
        match self.store.run_at(idx) {
            Some(run) => (
                mix_frame(m, [run.vocals[0], run.vocals[1]], [run.drums[0], run.drums[1]], g),
                true,
            ),
            None => (m, false),
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
        let last = self.store.frames().saturating_sub(1);
        let mut produced = 0;

        for f in 0..out_frames {
            let base = f * channels;
            if self.cursor >= self.len_frames {
                out[base..base + channels].fill(0.0);
                continue;
            }
            let pos = self.cursor as f64 * self.step;
            let i0 = (pos.floor() as usize).min(last);
            let i1 = (i0 + 1).min(last);
            let frac = (pos - i0 as f64) as f32;

            // Gains glide toward the setting where stems exist, and toward
            // unity (the untouched original) where they don't.
            let (a, ready_a) = self.frame_at(i0, self.smoothed);
            let (b, _) = self.frame_at(i1, self.smoothed);
            for (sm, &t) in self.smoothed.iter_mut().zip(target.iter()) {
                let goal = if ready_a { t } else { 1.0 };
                *sm += (goal - *sm) * GAIN_SMOOTH;
                if (goal - *sm).abs() < 1e-6 {
                    *sm = goal;
                }
            }
            let l = a[0] + (b[0] - a[0]) * frac;
            let r = a[1] + (b[1] - a[1]) * frac;

            self.cursor += 1;
            produced += 1;
            if channels == 1 {
                out[base] = 0.5 * (l + r);
            } else {
                out[base] = l;
                out[base + 1] = r;
                out[base + 2..base + channels].fill(0.0);
            }
        }
        self.store.set_wanted((self.cursor as f64 * self.step) as usize);
        produced
    }

    fn stop(&mut self) {
        self.cursor = self.len_frames;
    }

    fn seek(&mut self, frame: usize) {
        self.cursor = frame.min(self.len_frames);
        self.store.set_wanted((self.cursor as f64 * self.step) as usize);
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
    use hm_stems::chunk::ChunkOut;

    /// A store with a short track whose single chunk is ready: mix = `m`,
    /// vocals = `v`, drums = `d` (all per-sample constants, stereo).
    fn ready_store(frames: usize, m: f32, v: f32, d: f32) -> Arc<StemStore> {
        let store = StemStore::new(vec![m; frames * 2]);
        assert!(store.publish(0, ChunkOut { vocals: vec![v; frames * 2], drums: vec![d; frames * 2] }));
        store
    }

    fn play(src: &mut StemPlaybackSource, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        let n = src.read(&mut out, 2);
        assert_eq!(n, frames);
        out
    }

    fn settled(store: Arc<StemStore>, gains: [f32; 3]) -> StemPlaybackSource {
        let g = Arc::new(StemGains::default());
        let mut src = StemPlaybackSource::new(store, g.clone(), 44_100);
        for (i, v) in gains.iter().enumerate() {
            g.set(i, *v);
        }
        play(&mut src, 6000); // let the smoothing converge
        src.seek(0);
        src
    }

    #[test]
    fn mix_formula_matches_the_mobile_definition() {
        // M = 1.0, V = 0.3, D = 0.2 => I = 0.5.
        let (m, v, d) = ([1.0, 1.0], [0.3, 0.3], [0.2, 0.2]);
        let near = |a: [f32; 2], x: f32| assert!((a[0] - x).abs() < 1e-6 && (a[1] - x).abs() < 1e-6, "{a:?} != {x}");
        near(mix_frame(m, v, d, [1.0, 1.0, 1.0]), 1.0);
        near(mix_frame(m, v, d, [0.0, 1.0, 1.0]), 0.7); // no vocals
        near(mix_frame(m, v, d, [1.0, 0.0, 1.0]), 0.8); // no drums
        near(mix_frame(m, v, d, [1.0, 1.0, 0.0]), 0.5); // no instruments
        near(mix_frame(m, v, d, [0.0, 0.0, 0.0]), 0.0);
        near(mix_frame(m, v, d, [2.0, 0.5, 0.0]), 2.0 * 0.3 + 0.5 * 0.2);
    }

    #[test]
    fn unity_is_the_original_bit_for_bit() {
        // Awkward floats where (v + d) + (m - v - d) != m in general.
        let frames = 4000;
        let mix: Vec<f32> = (0..frames * 2).map(|i| ((i as f32) * 0.37).sin() * 0.9 + 1e-7 * i as f32).collect();
        let store = StemStore::new(mix.clone());
        let v: Vec<f32> = mix.iter().map(|x| x * 0.3137 + 1e-4).collect();
        let d: Vec<f32> = mix.iter().map(|x| x * -0.2111 + 3e-5).collect();
        store.publish(0, ChunkOut { vocals: v, drums: d });
        let mut src = StemPlaybackSource::new(store, Arc::new(StemGains::default()), 44_100);
        let out = play(&mut src, frames - 1); // last frame lerps with its clamped neighbour
        assert!(out.iter().zip(&mix).all(|(a, b)| a.to_bits() == b.to_bits()), "not bit-exact");
    }

    #[test]
    fn each_control_scales_only_its_stem() {
        let store = ready_store(20_000, 1.0, 0.3, 0.2);
        let probe = |gains| -> f32 { play(&mut settled(store.clone(), gains), 100)[50] };
        assert!((probe([0.0, 1.0, 1.0]) - 0.7).abs() < 1e-4);
        assert!((probe([1.0, 0.0, 1.0]) - 0.8).abs() < 1e-4);
        assert!((probe([1.0, 1.0, 0.0]) - 0.5).abs() < 1e-4);
        assert!((probe([0.0, 0.0, 1.0]) - 0.5).abs() < 1e-4); // instruments solo
        assert!((probe([1.0, 0.0, 0.0]) - 0.3).abs() < 1e-4); // vocals solo
    }

    #[test]
    fn unseparated_regions_play_the_original_whatever_the_gains() {
        // Chunk 0 not published at all.
        let store = StemStore::new(vec![0.6; 20_000 * 2]);
        let mut src = settled(store, [0.0, 0.0, 0.0]);
        let out = play(&mut src, 500);
        assert!(out.iter().all(|&x| (x - 0.6).abs() < 1e-6), "should be untouched");
    }

    #[test]
    fn gains_glide_in_once_the_chunk_arrives() {
        let frames = 30_000;
        let store = StemStore::new(vec![1.0; frames * 2]);
        let gains = Arc::new(StemGains::default());
        gains.set(EL_VOCALS, 0.0);
        let mut src = StemPlaybackSource::new(store.clone(), gains, 44_100);
        // While unseparated the gains drift to unity whatever the setting was.
        assert!((play(&mut src, 6000)[0] - 1.0).abs() < 1e-6);
        src.seek(0);
        store.publish(0, ChunkOut { vocals: vec![0.4; frames * 2], drums: vec![0.0; frames * 2] });
        let a = play(&mut src, 8)[0];
        assert!(a > 0.99, "no step: {a}"); // starts at unity
        play(&mut src, 6000);
        let z = play(&mut src, 8)[0];
        assert!((z - 0.6).abs() < 1e-3, "vocals removed once settled: {z}");
    }

    #[test]
    fn reports_where_the_listener_is_so_the_worker_can_follow() {
        let store = ready_store(20_000, 0.5, 0.0, 0.0);
        let mut src = StemPlaybackSource::new(store.clone(), Arc::new(StemGains::default()), 44_100);
        play(&mut src, 1234);
        assert_eq!(store.wanted(), 1234);
        src.seek(9000);
        assert_eq!(store.wanted(), 9000);
        assert_eq!(src.position(), 9000);
    }

    #[test]
    fn resamples_to_the_device_rate_like_the_plain_file_path() {
        // 44.1k ramp played on a 48k device: same length and linear values as
        // hm_audio::resample_stereo.
        let frames = 4410;
        let mix: Vec<f32> = (0..frames).flat_map(|i| [i as f32 / 10_000.0, -(i as f32) / 10_000.0]).collect();
        let store = StemStore::new(mix.clone());
        let mut src = StemPlaybackSource::new(store, Arc::new(StemGains::default()), 48_000);
        assert_eq!(src.total_frames(), 4800);
        let out = play(&mut src, 4800);
        let reference = crate::resample_stereo(&mix, 44_100, 48_000);
        assert_eq!(reference.len(), out.len());
        for (a, b) in out.iter().zip(&reference).take(out.len() - 4) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn ends_and_goes_silent_after_the_last_frame() {
        let store = ready_store(100, 0.5, 0.0, 0.0);
        let mut src = StemPlaybackSource::new(store, Arc::new(StemGains::default()), 44_100);
        let mut out = vec![1.0f32; 300 * 2];
        assert_eq!(src.read(&mut out, 2), 100);
        assert!(out[200..].iter().all(|&x| x == 0.0));
        assert_eq!(src.position(), 100);
    }

    #[test]
    fn swap_lands_on_the_playhead_in_the_device_domain() {
        assert_eq!(swap_start_frame(12.5, 48_000, 10_000_000), 600_000);
        assert_eq!(swap_start_frame(12.5, 44_100, 10_000_000), 551_250);
        assert_eq!(swap_start_frame(-3.0, 48_000, 100), 0);
        assert_eq!(swap_start_frame(1e9, 48_000, 100), 100, "clamped to the end");
    }

    #[test]
    fn a_swap_at_the_playhead_continues_from_the_same_sample() {
        // A ramp: after seeking to the swap frame the first output is the ramp
        // value at that instant, not the start of the track.
        let frames = 44_100;
        let mix: Vec<f32> = (0..frames).flat_map(|i| [i as f32 / frames as f32; 2]).collect();
        let store = StemStore::new(mix);
        let mut src = StemPlaybackSource::new(store, Arc::new(StemGains::default()), 44_100);
        src.seek(swap_start_frame(0.5, 44_100, src.total_frames()));
        let first = play(&mut src, 1)[0];
        assert!((first - 0.5).abs() < 1e-4, "{first}");
    }

    #[test]
    fn non_finite_gains_fall_back_to_unity() {
        let g = StemGains::default();
        g.set(0, f32::NAN);
        g.set(1, 9.0);
        assert_eq!(g.get(0), 1.0);
        assert_eq!(g.get(1), 2.0);
        assert_eq!(g.get(7), 0.0);
    }
}
