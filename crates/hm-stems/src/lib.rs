//! Real-time stem separation via **htdemucs_ft** (Demucs v4, fine-tuned) running
//! in-process on ONNX Runtime — VirtualDJ-grade clean stems, GPU-accelerated.
//!
//! Uses StemSplit's parity-verified ONNX export: **four per-stem specialist
//! models** (drums, bass, other, vocals), each waveform→waveform with the
//! STFT/iSTFT and normalization baked into the graph (so no PyTorch and no
//! host-side spectral maths). On Apple Silicon CoreML drives them on the Neural
//! Engine, so the player separates a loaded track in a few seconds and the UI's
//! pads feel instant. The mixing — live, click-free per-element gains with the
//! drum stem split into kick/hihat — lives in `hm-audio::stems`.
//!
//! Pipeline (mirrors the repo's `bag_infer.py`): decode → resample to 44.1 kHz →
//! split into overlapping segments → run all 4 specialists per segment, keeping
//! each one's own source row → trapezoidal **overlap-add** → four interleaved-
//! stereo stem buffers. Results cache to disk per source file.

mod onnx;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use onnx::Model;

/// Number of separated stems (vocals, drums, bass, other).
pub const STEM_COUNT: usize = 4;
/// htdemucs operates at 44.1 kHz; every stem buffer comes back at this rate.
pub const STEM_SR: u32 = 44_100;
/// Overlap fraction between adjacent inference segments (htdemucs default).
const OVERLAP_DIV: usize = 4; // overlap = segment / 4

/// Output stem names in playback-slot order (slot 0 vocals … 3 other).
const STEM_NAMES: [&str; STEM_COUNT] = ["vocals", "drums", "bass", "other"];

/// The four specialist models: `(filename, output row this model is trusted for,
/// destination playback slot)`. htdemucs' native source order is
/// `[drums, bass, other, vocals]`, so each specialist's row index differs from
/// our slot order `[vocals, drums, bass, other]`.
const MODELS: [(&str, usize, usize); STEM_COUNT] = [
    ("htdemucs_ft_vocals.onnx", 3, 0), // vocals row 3 → slot 0
    ("htdemucs_ft_drums.onnx", 0, 1),  // drums  row 0 → slot 1
    ("htdemucs_ft_bass.onnx", 1, 2),   // bass   row 1 → slot 2
    ("htdemucs_ft_other.onnx", 2, 3),  // other  row 2 → slot 3
];

/// Four separated stems, interleaved stereo at [`STEM_SR`], in slot order
/// (0 vocals, 1 drums, 2 bass, 3 other).
pub type StemSet = [Vec<f32>; STEM_COUNT];

#[derive(Debug, thiserror::Error)]
pub enum StemError {
    #[error("the stem models aren't installed yet (run scripts/get_stems_model.sh)")]
    Unavailable,
    #[error("stem engine error: {0}")]
    Engine(String),
    #[error("io error: {0}")]
    Io(String),
    /// Separation was cancelled mid-run (the user armed a different track or
    /// left the Stems view) — not a failure; callers treat it as a silent no-op.
    #[error("stem separation aborted")]
    Aborted,
}

/// In-process htdemucs_ft separator with on-disk result caching. The four ONNX
/// sessions are loaded lazily on first use and reused.
pub struct Separator {
    model_dir: PathBuf,
    cache_dir: PathBuf,
    models: Mutex<Option<Vec<Model>>>,
}

impl Separator {
    pub fn new(model_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            model_dir,
            cache_dir,
            models: Mutex::new(None),
        }
    }

    /// Whether all four model files are present (the separator can run).
    pub fn available(&self) -> bool {
        MODELS
            .iter()
            .all(|(file, _, _)| self.model_dir.join(file).is_file())
    }

    /// Whether CoreML acceleration will be used (opt-in via `HM_STEMS_COREML`,
    /// and only if the runtime has the EP) — cheap, doesn't load the models.
    pub fn accelerated(&self) -> bool {
        std::env::var("HM_STEMS_COREML").is_ok() && onnx::coreml_available()
    }

    fn cache_slot(&self, input: &Path) -> PathBuf {
        self.cache_dir.join(cache_key(input))
    }

    /// Load this track's stems from the cache, if present.
    pub fn cached(&self, input: &Path) -> Option<StemSet> {
        let dir = self.cache_slot(input);
        let mut set: StemSet = Default::default();
        for (i, name) in STEM_NAMES.iter().enumerate() {
            set[i] = read_wav_f32(&dir.join(format!("{name}.wav")))?;
        }
        Some(set)
    }

    fn ensure_loaded<'a>(
        &self,
        guard: &'a mut Option<Vec<Model>>,
    ) -> Result<&'a mut Vec<Model>, StemError> {
        if guard.is_none() {
            if !self.available() {
                return Err(StemError::Unavailable);
            }
            let mut models = Vec::with_capacity(STEM_COUNT);
            for (file, row, _slot) in MODELS {
                models.push(Model::load(&self.model_dir.join(file), row)?);
            }
            *guard = Some(models);
        }
        Ok(guard.as_mut().expect("just loaded"))
    }

    /// Separate `mix` (interleaved **stereo** f32 at 44.1 kHz) into four stems,
    /// caching the result under `input`'s key. `on_progress` receives 0.0..=1.0.
    /// Blocking and CPU/GPU-heavy — run off the audio + UI threads.
    pub fn separate(
        &self,
        mix: &[f32],
        input: &Path,
        on_progress: impl Fn(f32),
    ) -> Result<StemSet, StemError> {
        self.separate_cancellable(mix, input, on_progress, &AtomicBool::new(false))
    }

    /// [`separate`](Self::separate), but abortable: `cancel` is checked before
    /// every model run, so a separation the user has abandoned (armed another
    /// track, closed the Stems view) stops within one segment instead of
    /// grinding all 4 models × ~41 segments to completion on a weak CPU.
    /// Returns [`StemError::Aborted`] when cancelled; nothing is cached.
    pub fn separate_cancellable(
        &self,
        mix: &[f32],
        input: &Path,
        on_progress: impl Fn(f32),
        cancel: &AtomicBool,
    ) -> Result<StemSet, StemError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(StemError::Aborted);
        }
        if let Some(cached) = self.cached(input) {
            on_progress(1.0);
            return Ok(cached);
        }

        let mut guard = self
            .models
            .lock()
            .map_err(|_| StemError::Engine("separator lock poisoned".into()))?;
        let models = self.ensure_loaded(&mut guard)?;
        let segment = models[0].segment;

        let frames = mix.len() / 2;
        if frames == 0 {
            return Err(StemError::Engine("empty input".into()));
        }

        // Planar channels (NO normalization — it's baked into the ONNX graph).
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for f in 0..frames {
            left[f] = mix[2 * f];
            right[f] = mix[2 * f + 1];
        }

        let window = transition_window(segment);
        let stride = (segment - segment / OVERLAP_DIV).max(1);

        // Per-slot, per-channel accumulators + shared per-sample weight sum.
        let mut acc: Vec<[Vec<f32>; 2]> = (0..STEM_COUNT)
            .map(|_| [vec![0.0f32; frames], vec![0.0f32; frames]])
            .collect();
        let mut wsum = vec![0.0f32; frames];

        let mut seg_in = vec![0.0f32; 2 * segment];
        let offsets: Vec<usize> = (0..frames).step_by(stride).collect();
        let total = offsets.len().max(1);

        for (si, &off) in offsets.iter().enumerate() {
            let take = segment.min(frames - off);
            seg_in.fill(0.0);
            seg_in[..take].copy_from_slice(&left[off..off + take]);
            seg_in[segment..segment + take].copy_from_slice(&right[off..off + take]);

            for (mi, (_, _, slot)) in MODELS.iter().enumerate() {
                // A model run is the smallest interruptible unit (an ONNX
                // inference can't be stopped mid-flight), so check here.
                if cancel.load(Ordering::Relaxed) {
                    return Err(StemError::Aborted);
                }
                let stem = models[mi].run_segment(&seg_in)?; // [2 × segment], channel-major
                let dst = &mut acc[*slot];
                for c in 0..2 {
                    let plane = &stem[c * segment..c * segment + take];
                    for i in 0..take {
                        dst[c][off + i] += plane[i] * window[i];
                    }
                }
            }
            for i in 0..take {
                wsum[off + i] += window[i];
            }
            on_progress(0.02 + (si + 1) as f32 / total as f32 * 0.96);
        }

        // Normalize by accumulated weight, re-interleave.
        let mut set: StemSet = Default::default();
        for (s, channels) in acc.into_iter().enumerate() {
            let mut inter = vec![0.0f32; frames * 2];
            for f in 0..frames {
                let w = if wsum[f] > 1e-8 { 1.0 / wsum[f] } else { 0.0 };
                inter[2 * f] = channels[0][f] * w;
                inter[2 * f + 1] = channels[1][f] * w;
            }
            set[s] = inter;
        }

        self.write_cache(input, &set);
        on_progress(1.0);
        Ok(set)
    }

    fn write_cache(&self, input: &Path, set: &StemSet) {
        let dir = self.cache_slot(input);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        for (i, name) in STEM_NAMES.iter().enumerate() {
            let _ = write_wav_f32(&dir.join(format!("{name}.wav")), &set[i]);
        }
    }
}

/// htdemucs_ft's overlap-add window (per `bag_infer.py`): flat 1.0 with linear
/// fades over the first/last `segment/4` samples — so segment seams cross-fade.
fn transition_window(segment: usize) -> Vec<f32> {
    let transition = (segment / OVERLAP_DIV).max(1);
    let mut w = vec![1.0f32; segment];
    for k in 0..transition.min(segment) {
        let v = if transition > 1 {
            k as f32 / (transition - 1) as f32
        } else {
            1.0
        };
        w[k] = v; // fade in
        w[segment - 1 - k] = v; // fade out (mirror)
    }
    w
}

/// A stable cache key for a source file: name + size + mtime, so re-encodes or
/// edits re-separate but identical files are reused.
fn cache_key(input: &Path) -> String {
    let name = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("track");
    let (size, mtime) = std::fs::metadata(input)
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (m.len(), mtime)
        })
        .unwrap_or((0, 0));
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect();
    format!("{sanitized}-{size}-{mtime}")
}

/// Write interleaved-stereo f32 samples to a 32-bit float WAV (cache).
fn write_wav_f32(path: &Path, samples: &[f32]) -> Result<(), StemError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: STEM_SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| StemError::Io(e.to_string()))?;
    for &s in samples {
        w.write_sample(s).map_err(|e| StemError::Io(e.to_string()))?;
    }
    w.finalize().map_err(|e| StemError::Io(e.to_string()))
}

/// Read a cached f32 WAV back to interleaved samples, or `None` if missing.
fn read_wav_f32(path: &Path) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).ok()?;
    reader.samples::<f32>().collect::<Result<Vec<_>, _>>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_without_models() {
        let s = Separator::new("/no/such/models".into(), std::env::temp_dir());
        assert!(!s.available());
        assert!(matches!(
            s.separate(&[0.0, 0.0], Path::new("/tmp/x.wav"), |_| {}),
            Err(StemError::Unavailable)
        ));
    }

    #[test]
    fn cancelled_separation_aborts_without_doing_work() {
        let s = Separator::new("/no/such/models".into(), std::env::temp_dir());
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            s.separate_cancellable(&[0.0, 0.0], Path::new("/tmp/x.wav"), |_| {}, &cancel),
            Err(StemError::Aborted)
        ));
    }

    #[test]
    fn cache_key_is_stable_and_sanitized() {
        let k = cache_key(Path::new("/music/My Song (2024)!.flac"));
        assert!(k.starts_with("My_Song__2024__"));
        assert!(!k.contains('/') && !k.contains(' '));
    }

    #[test]
    fn transition_window_is_trapezoidal() {
        let w = transition_window(8); // transition = 2
        assert_eq!(w.len(), 8);
        assert_eq!(w[0], 0.0); // fade starts at 0
        assert_eq!(w[7], 0.0); // and ends at 0
        assert_eq!(w[3], 1.0); // flat in the middle
        assert_eq!(w[4], 1.0);
        assert!(w.iter().all(|&x| (0.0..=1.0).contains(&x)));
    }

    #[test]
    fn model_table_maps_rows_to_distinct_slots() {
        let mut slots: Vec<usize> = MODELS.iter().map(|(_, _, s)| *s).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 2, 3]);
        // vocals specialist feeds slot 0.
        assert_eq!(MODELS[0], ("htdemucs_ft_vocals.onnx", 3, 0));
    }

    /// End-to-end run against the real downloaded models + ONNX Runtime dylib.
    /// Ignored by default (needs ~1.3 GB of models). Run with:
    ///   ORT_DYLIB_PATH=<…/stems/libonnxruntime.dylib> \
    ///   cargo test -p hm-stems -- --ignored --nocapture
    #[test]
    #[ignore = "needs the downloaded htdemucs_ft models + ORT dylib"]
    fn separates_real_models_end_to_end() {
        let home = std::env::var("HOME").unwrap();
        let stems_root = format!("{home}/Library/Application Support/com.hypemuzik.desktop/stems");
        if std::env::var("ORT_DYLIB_PATH").is_err() {
            std::env::set_var("ORT_DYLIB_PATH", format!("{stems_root}/libonnxruntime.dylib"));
        }
        let cache = std::env::temp_dir().join("hm_stems_test_cache");
        let _ = std::fs::remove_dir_all(&cache);
        let sep = Separator::new(format!("{stems_root}/model").into(), cache);
        assert!(sep.available(), "models not found under {stems_root}/model");

        // 2 s of stereo: L 220 Hz, R 440 Hz.
        let n = STEM_SR as usize * 2;
        let mut mix = vec![0.0f32; n * 2];
        for f in 0..n {
            let t = f as f32 / STEM_SR as f32;
            mix[2 * f] = 0.2 * (std::f32::consts::TAU * 220.0 * t).sin();
            mix[2 * f + 1] = 0.2 * (std::f32::consts::TAU * 440.0 * t).sin();
        }
        let input = std::env::temp_dir().join("hm_stems_test_input.wav");
        std::fs::write(&input, b"test").unwrap();

        let last = std::cell::Cell::new(0.0f32);
        let stems = sep
            .separate(&mix, &input, |p| last.set(p))
            .expect("separation failed");

        assert_eq!(stems.len(), STEM_COUNT);
        let mut total_energy = 0.0f64;
        for (i, s) in stems.iter().enumerate() {
            assert_eq!(s.len(), n * 2, "stem {i} wrong length");
            assert!(s.iter().all(|x| x.is_finite()), "stem {i} has non-finite samples");
            total_energy += s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>();
        }
        assert_eq!(last.get(), 1.0);
        assert!(total_energy > 0.0, "all stems are silent");
        println!("end-to-end OK: 4 stems × {} frames, total energy {total_energy:.3}", n);
    }
}
