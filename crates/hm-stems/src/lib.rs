//! Offline stem separation via **Demucs v4** (htdemucs) — VirtualDJ-grade clean
//! stems (~9 dB SDR vs ~3-5 dB for DSP tricks).
//!
//! The neural separation runs in a native **sidecar** (`hm-demucs`, built from
//! `demucs.cpp` — ggml/CPU, ~81 MB f16 model) so the heavy ML/ggml build stays
//! out of the main app. This crate orchestrates it: locate the sidecar + model,
//! cache results per source file + mode, run the separation with progress, and
//! hand back the stem WAVs mapped to playback slots. Playback/mixing lives in
//! `hm-audio` (`StemPlaybackSource`).
//!
//! ## Sidecar contract
//!
//! ```text
//! hm-demucs --model <dir> --input <wav> --out <dir> --stems <2|4>
//! ```
//! prints `progress=<0.0..1.0>` lines to stdout, exits 0 on success, and writes
//! (4-stem) `vocals/drums/bass/other.wav` or (2-stem) `vocals/instrumental.wav`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// How many stems to separate into. Two-stem is roughly 2× faster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StemMode {
    /// Vocals, drums, bass, other → playback slots 0,1,2,3.
    Four,
    /// Vocals + instrumental → playback slots 0,1.
    Two,
}

impl StemMode {
    fn arg(self) -> &'static str {
        match self {
            StemMode::Four => "4",
            StemMode::Two => "2",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            StemMode::Four => "4stem",
            StemMode::Two => "2stem",
        }
    }

    /// The output file (without `.wav`) → playback slot mapping for this mode.
    fn layout(self) -> &'static [(&'static str, usize)] {
        match self {
            StemMode::Four => &[("vocals", 0), ("drums", 1), ("bass", 2), ("other", 3)],
            StemMode::Two => &[("vocals", 0), ("instrumental", 1)],
        }
    }
}

/// The separated stem WAVs, each mapped to a playback slot (0=vocals…).
#[derive(Debug, Clone, Serialize)]
pub struct Stems {
    pub mode: StemMode,
    pub stems: Vec<SlotPath>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotPath {
    pub slot: usize,
    pub path: PathBuf,
}

impl Stems {
    fn from_dir(dir: &Path, mode: StemMode) -> Self {
        let stems = mode
            .layout()
            .iter()
            .map(|(name, slot)| SlotPath {
                slot: *slot,
                path: dir.join(format!("{name}.wav")),
            })
            .collect();
        Self { mode, stems }
    }

    fn all_exist(&self) -> bool {
        self.stems.iter().all(|s| s.path.exists())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StemError {
    #[error("the stem separator isn't installed (build hm-demucs + the model)")]
    Unavailable,
    #[error("separation failed: {0}")]
    Failed(String),
    #[error("io error: {0}")]
    Io(String),
}

/// Orchestrates the Demucs sidecar with per-(file, mode) result caching.
pub struct Demucs {
    sidecar: PathBuf,
    model_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Demucs {
    pub fn new(sidecar: PathBuf, model_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            sidecar,
            model_dir,
            cache_dir,
        }
    }

    /// Whether the sidecar binary and the model are both present.
    pub fn available(&self) -> bool {
        self.sidecar.exists() && self.model_dir.exists()
    }

    fn cache_slot(&self, input: &Path, mode: StemMode) -> PathBuf {
        self.cache_dir
            .join(format!("{}-{}", cache_key(input), mode.slug()))
    }

    /// Stems for an already-separated source+mode, if cached.
    pub fn cached(&self, input: &Path, mode: StemMode) -> Option<Stems> {
        let stems = Stems::from_dir(&self.cache_slot(input, mode), mode);
        stems.all_exist().then_some(stems)
    }

    /// Separate `input` (a stereo WAV) into stems for `mode`, reusing the cache.
    /// `on_progress` gets 0.0..=1.0. Blocking — run off the UI thread.
    pub fn separate(
        &self,
        input: &Path,
        mode: StemMode,
        on_progress: impl Fn(f32),
    ) -> Result<Stems, StemError> {
        if !self.available() {
            return Err(StemError::Unavailable);
        }
        if let Some(cached) = self.cached(input, mode) {
            on_progress(1.0);
            return Ok(cached);
        }

        let out_dir = self.cache_slot(input, mode);
        std::fs::create_dir_all(&out_dir).map_err(|e| StemError::Io(e.to_string()))?;

        let mut child = Command::new(&self.sidecar)
            .arg("--model")
            .arg(&self.model_dir)
            .arg("--input")
            .arg(input)
            .arg("--out")
            .arg(&out_dir)
            .arg("--stems")
            .arg(mode.arg())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| StemError::Io(e.to_string()))?;

        if let Some(stdout) = child.stdout.take() {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(rest) = line.trim().strip_prefix("progress=") {
                    if let Ok(p) = rest.parse::<f32>() {
                        on_progress(p.clamp(0.0, 1.0));
                    }
                }
            }
        }

        let status = child.wait().map_err(|e| StemError::Io(e.to_string()))?;
        let stems = Stems::from_dir(&out_dir, mode);
        if !status.success() || !stems.all_exist() {
            let _ = std::fs::remove_dir_all(&out_dir);
            return Err(StemError::Failed(format!(
                "separator exited with {status} or produced no stems"
            )));
        }
        on_progress(1.0);
        Ok(stems)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_without_sidecar() {
        let d = Demucs::new(
            "/no/such/hm-demucs".into(),
            "/no/such/model".into(),
            std::env::temp_dir(),
        );
        assert!(!d.available());
        assert!(matches!(
            d.separate(Path::new("/tmp/x.wav"), StemMode::Four, |_| {}),
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
    fn mode_layouts_map_to_slots() {
        assert_eq!(StemMode::Four.layout().len(), 4);
        assert_eq!(StemMode::Two.layout(), &[("vocals", 0), ("instrumental", 1)]);
    }
}
