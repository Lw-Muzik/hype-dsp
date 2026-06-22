//! The ONNX Runtime wrapper for htdemucs — **all `ort`-version-specific code
//! lives here** so the rest of the crate is engine-agnostic.
//!
//! htdemucs is a waveform→waveform model: it takes a stereo mixture segment
//! `[1, 2, L]` and returns four source segments `[1, 4, 2, L]`
//! (vocals/drums/bass/other). The STFT/iSTFT and transformer live *inside* the
//! exported graph, so from here it's just tensor-in/tensor-out. We run it on the
//! **CoreML** execution provider (Apple Neural Engine / GPU) when available, and
//! fall back to CPU automatically.
//!
//! The model's I/O contract (input name, output name, segment length `L`) is
//! read from a sidecar `<model>.json` written by `scripts/export_demucs_onnx.py`
//! so we never have to introspect ONNX shapes (whose Rust API churns). Sensible
//! htdemucs defaults are used if the JSON is absent.

use std::path::Path;

use ort::execution_providers::{CoreMLExecutionProvider, ExecutionProvider};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

use crate::StemError;

/// htdemucs trains on 7.8 s segments → 7.8 × 44100 = 343 980 samples.
const DEFAULT_SEGMENT: usize = 343_980;

#[derive(Deserialize)]
struct Contract {
    #[serde(default = "default_input")]
    input: String,
    #[serde(default = "default_output")]
    output: String,
    #[serde(default = "default_segment")]
    segment: usize,
}
fn default_input() -> String {
    "mix".into()
}
fn default_output() -> String {
    "stems".into()
}
fn default_segment() -> usize {
    DEFAULT_SEGMENT
}
impl Default for Contract {
    fn default() -> Self {
        Self {
            input: default_input(),
            output: default_output(),
            segment: default_segment(),
        }
    }
}

/// A loaded htdemucs ONNX model ready to separate segments.
pub struct Model {
    session: Session,
    input: String,
    output: String,
    /// Samples per inference segment (per channel).
    pub segment: usize,
    /// Whether CoreML (ANE/GPU) is driving inference, vs. the CPU fallback.
    pub accelerated: bool,
}

impl Model {
    /// Load `path` (an htdemucs `.onnx`), preferring the CoreML EP. Reads
    /// `<path>.json` for the I/O contract; falls back to htdemucs defaults.
    pub fn load(path: &Path) -> Result<Self, StemError> {
        let contract: Contract = std::fs::read_to_string(with_json_ext(path))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let builder = Session::builder().map_err(ort_err)?;

        // Prefer CoreML (Neural Engine / GPU); the runtime silently falls back
        // to CPU for any op CoreML can't run, and we drop the EP entirely if the
        // loaded libonnxruntime wasn't built with CoreML.
        let coreml = CoreMLExecutionProvider::default();
        let accelerated = coreml.is_available().unwrap_or(false);
        let builder = if accelerated {
            builder
                .with_execution_providers([coreml.build()])
                .map_err(ort_err)?
        } else {
            builder
        };

        let session = builder.commit_from_file(path).map_err(ort_err)?;
        Ok(Self {
            session,
            input: contract.input,
            output: contract.output,
            segment: contract.segment.max(1),
            accelerated,
        })
    }

    /// Separate one segment. `planar` is `[2 × segment]` laid out channel-major
    /// (all of L, then all of R). Returns `[4 × 2 × segment]` in source-major,
    /// channel, sample order (htdemucs' native output layout).
    pub fn run_segment(&mut self, planar: &[f32]) -> Result<Vec<f32>, StemError> {
        debug_assert_eq!(planar.len(), 2 * self.segment);
        let tensor = Tensor::from_array(([1usize, 2, self.segment], planar.to_vec()))
            .map_err(ort_err)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => tensor])
            .map_err(ort_err)?;
        // `try_extract_tensor` hands back the contiguous C-order buffer directly
        // (source-major: [4 × 2 × segment]) — no ndarray needed.
        let (_, data) = outputs[self.output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(ort_err)?;
        Ok(data.to_vec())
    }
}

fn with_json_ext(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".json");
    std::path::PathBuf::from(s)
}

fn ort_err(e: impl std::fmt::Display) -> StemError {
    StemError::Engine(e.to_string())
}
