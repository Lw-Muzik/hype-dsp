//! Real-time stem separation via **htdemucs** (Demucs v4) running in-process on
//! ONNX Runtime — VirtualDJ-grade clean stems (~9 dB SDR), GPU-accelerated.
//!
//! On Apple Silicon htdemucs separates ~30× faster than real-time (a 4-minute
//! track in a few seconds), so the player can separate a track **the moment it
//! loads**, transparently, and the UI's faders feel instant — exactly how
//! VirtualDJ behaves on capable hardware (it pre-renders to disk only as a
//! fallback for weak machines). The actual mixing — live, click-free per-stem
//! gains driving the faders/mute/solo — lives in `hm-audio::stems`.
//!
//! Pipeline: decode → resample to 44.1 kHz → **normalize** (htdemucs expects a
//! zero-mean/unit-std mixture) → split into overlapping segments → run each on
//! CoreML (auto CPU fallback) → triangular **overlap-add** → denormalize → four
//! interleaved-stereo stem buffers. Results are cached to disk per source file
//! so re-opening a track is instant.
//!
//! "2-stem" vs "4-stem" is **not** a separate (slower) run: htdemucs always
//! emits four sources in one pass, so the UI simply regroups them
//! (vocals / everything-else) — switching modes is free.

mod onnx;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use onnx::Model;

/// The four htdemucs stems, in their fixed playback-slot order.
pub const STEM_COUNT: usize = 4;
/// htdemucs operates at 44.1 kHz; every stem buffer comes back at this rate.
pub const STEM_SR: u32 = 44_100;
/// Overlap fraction between adjacent inference segments (htdemucs default).
const OVERLAP: f32 = 0.25;

const STEM_NAMES: [&str; STEM_COUNT] = ["vocals", "drums", "bass", "other"];

/// Four separated stems, interleaved stereo at [`STEM_SR`], in slot order
/// (0 vocals, 1 drums, 2 bass, 3 other).
pub type StemSet = [Vec<f32>; STEM_COUNT];

#[derive(Debug, thiserror::Error)]
pub enum StemError {
    #[error("the stem separator isn't installed yet (htdemucs.onnx + libonnxruntime)")]
    Unavailable,
    #[error("stem engine error: {0}")]
    Engine(String),
    #[error("io error: {0}")]
    Io(String),
}

/// In-process htdemucs separator with on-disk result caching. The ONNX session
/// is loaded lazily on first use and reused (cheap to keep resident).
pub struct Separator {
    model_path: PathBuf,
    cache_dir: PathBuf,
    model: Mutex<Option<Model>>,
}

impl Separator {
    pub fn new(model_path: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            model_path,
            cache_dir,
            model: Mutex::new(None),
        }
    }

    /// Whether the model file is present (the separator can run).
    pub fn available(&self) -> bool {
        self.model_path.is_file()
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

    /// Whether CoreML acceleration is active (vs. CPU). Loads the model if
    /// needed; `None` if it can't be loaded.
    pub fn accelerated(&self) -> Option<bool> {
        let mut guard = self.model.lock().ok()?;
        self.ensure_loaded(&mut guard).ok()?;
        guard.as_ref().map(|m| m.accelerated)
    }

    fn ensure_loaded<'a>(
        &self,
        guard: &'a mut Option<Model>,
    ) -> Result<&'a mut Model, StemError> {
        if guard.is_none() {
            if !self.available() {
                return Err(StemError::Unavailable);
            }
            *guard = Some(Model::load(&self.model_path)?);
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
        if let Some(cached) = self.cached(input) {
            on_progress(1.0);
            return Ok(cached);
        }

        let mut guard = self.model.lock().map_err(|_| {
            StemError::Engine("separator lock poisoned".into())
        })?;
        let model = self.ensure_loaded(&mut guard)?;
        let segment = model.segment;

        let frames = mix.len() / 2;
        if frames == 0 {
            return Err(StemError::Engine("empty input".into()));
        }

        // --- Normalize: htdemucs expects a zero-mean / unit-std mixture. -----
        let (mean, std) = mono_mean_std(mix, frames);
        let inv_std = if std > 1e-8 { 1.0 / std } else { 1.0 };
        // Planar, normalized channels.
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for f in 0..frames {
            left[f] = (mix[2 * f] - mean) * inv_std;
            right[f] = (mix[2 * f + 1] - mean) * inv_std;
        }

        // --- Overlap-add segmentation. --------------------------------------
        let weight = triangular_weight(segment);
        let stride = (((1.0 - OVERLAP) * segment as f32) as usize).max(1);

        // Per-source, per-channel accumulators + shared per-sample weight sum.
        let mut acc: Vec<[Vec<f32>; 2]> = (0..STEM_COUNT)
            .map(|_| [vec![0.0f32; frames], vec![0.0f32; frames]])
            .collect();
        let mut wsum = vec![0.0f32; frames];

        let mut seg_in = vec![0.0f32; 2 * segment];
        let offsets: Vec<usize> = (0..frames).step_by(stride).collect();
        let total = offsets.len().max(1);

        for (si, &off) in offsets.iter().enumerate() {
            let take = segment.min(frames - off);
            // Build planar [L(segment), R(segment)], zero-padded past `take`.
            seg_in[..segment].fill(0.0);
            seg_in[segment..].fill(0.0);
            seg_in[..take].copy_from_slice(&left[off..off + take]);
            seg_in[segment..segment + take].copy_from_slice(&right[off..off + take]);

            let out = model.run_segment(&seg_in)?; // [4 × 2 × segment], source-major

            for s in 0..STEM_COUNT {
                for c in 0..2 {
                    let plane = &out[(s * 2 + c) * segment..(s * 2 + c) * segment + take];
                    let dst = &mut acc[s][c][off..off + take];
                    for i in 0..take {
                        dst[i] += plane[i] * weight[i];
                    }
                }
            }
            for i in 0..take {
                wsum[off + i] += weight[i];
            }
            on_progress(0.02 + (si + 1) as f32 / total as f32 * 0.96);
        }

        // --- Normalize by accumulated weight, denormalize, re-interleave. ----
        let mut set: StemSet = Default::default();
        for s in 0..STEM_COUNT {
            let mut inter = vec![0.0f32; frames * 2];
            for f in 0..frames {
                let w = if wsum[f] > 1e-8 { 1.0 / wsum[f] } else { 0.0 };
                inter[2 * f] = acc[s][0][f] * w * std + mean;
                inter[2 * f + 1] = acc[s][1][f] * w * std + mean;
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

/// Mean and standard deviation of the channel-averaged (mono) mixture.
fn mono_mean_std(mix: &[f32], frames: usize) -> (f32, f32) {
    let mut sum = 0.0f64;
    for f in 0..frames {
        sum += 0.5 * (mix[2 * f] as f64 + mix[2 * f + 1] as f64);
    }
    let mean = (sum / frames as f64) as f32;
    let mut var = 0.0f64;
    for f in 0..frames {
        let m = 0.5 * (mix[2 * f] as f64 + mix[2 * f + 1] as f64) - mean as f64;
        var += m * m;
    }
    let std = (var / frames as f64).sqrt() as f32;
    (mean, std)
}

/// htdemucs' triangular overlap-add window: ramps 1→mid up then mid→1 down, so
/// the seam between segments is cross-faded and edges stay weighted (never 0).
fn triangular_weight(segment: usize) -> Vec<f32> {
    let half = segment / 2;
    let mut w = vec![0.0f32; segment];
    for (i, wi) in w.iter_mut().enumerate() {
        *wi = if i < half {
            (i + 1) as f32
        } else {
            (segment - i) as f32
        };
    }
    let max = w.iter().cloned().fold(1.0f32, f32::max);
    for wi in &mut w {
        *wi /= max;
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
    fn unavailable_without_model() {
        let s = Separator::new("/no/such/htdemucs.onnx".into(), std::env::temp_dir());
        assert!(!s.available());
        assert!(matches!(
            s.separate(&[0.0, 0.0], Path::new("/tmp/x.wav"), |_| {}),
            Err(StemError::Unavailable)
        ));
    }

    #[test]
    fn cache_key_is_stable_and_sanitized() {
        let k = cache_key(Path::new("/music/My Song (2024)!.flac"));
        assert!(k.starts_with("My_Song__2024__"));
        assert!(!k.contains('/') && !k.contains(' '));
    }

    #[test]
    fn triangular_weight_peaks_in_the_middle_and_is_positive() {
        let w = triangular_weight(8);
        assert_eq!(w.len(), 8);
        assert!(w.iter().all(|&x| x > 0.0));
        let max_i = w
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!((3..=4).contains(&max_i), "peak at {max_i}");
        assert!(w[0] < w[3] && w[7] < w[4]); // edges below the peak
    }

    #[test]
    fn mono_stats_match_known_signal() {
        // L/R both constant 1.0 → mono mean 1.0, std 0.0.
        let (mean, std) = mono_mean_std(&[1.0, 1.0, 1.0, 1.0], 2);
        assert!((mean - 1.0).abs() < 1e-6);
        assert!(std < 1e-6);
    }
}
