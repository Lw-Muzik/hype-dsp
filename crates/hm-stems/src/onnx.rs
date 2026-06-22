//! The ONNX Runtime wrapper for htdemucs_ft — **all `ort`-version-specific code
//! lives here** so the rest of the crate is engine-agnostic.
//!
//! We use StemSplit's parity-verified **htdemucs_ft** ONNX export: four per-stem
//! "specialist" models (drums, bass, other, vocals). Each is waveform→waveform —
//! it takes a stereo mixture segment `[1, 2, L]` and returns `[1, 4, 2, L]`
//! (the STFT/iSTFT and normalization live *inside* the graph), but each
//! specialist is only trusted for its own source row, so we extract that one row
//! (see `bag_infer.py` in the model repo). We run on **CoreML** (Apple Neural
//! Engine / GPU) when available, falling back to CPU automatically.

use std::path::Path;

use ort::execution_providers::{CoreMLExecutionProvider, ExecutionProvider};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

use crate::StemError;

/// htdemucs trains on 7.8 s segments → 7.8 × 44100 = 343 980 samples.
pub const SEGMENT: usize = 343_980;

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
    SEGMENT
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

/// Is the CoreML EP available in the loaded ONNX Runtime? Cheap — queries the
/// runtime's provider list without loading any model.
pub fn coreml_available() -> bool {
    CoreMLExecutionProvider::default()
        .is_available()
        .unwrap_or(false)
}

/// One loaded htdemucs_ft specialist model, trusted for a single source row.
pub struct Model {
    session: Session,
    input: String,
    output: String,
    /// Which row of the model's `[4, 2, L]` output is this specialist's source.
    target_row: usize,
    /// Samples per inference segment (per channel).
    pub segment: usize,
}

impl Model {
    /// Load an htdemucs_ft specialist `.onnx`, preferring the CoreML EP. Reads
    /// `<path>.json` for the I/O contract; falls back to htdemucs defaults
    /// (input `mix`, output `stems`, segment 343980). `target_row` selects this
    /// specialist's source from the model's 4-row output.
    pub fn load(path: &Path, target_row: usize) -> Result<Self, StemError> {
        let contract: Contract = std::fs::read_to_string(with_json_ext(path))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // CoreML is **opt-in**: it can't run htdemucs_ft on some Apple GPUs
        // (verified on M1 — `NeuralNetwork` hits an Espresso slice-compile bug and
        // `MLProgram` crashes at runtime with Metal command-buffer errors), so we
        // default to the reliable CPU path. `HM_STEMS_COREML=1` tries CoreML and
        // falls back to CPU if the session won't build.
        let want_coreml = std::env::var("HM_STEMS_COREML").is_ok() && coreml_available();
        let cpu_session = || -> Result<Session, StemError> {
            Session::builder()
                .map_err(ort_err)?
                .commit_from_file(path)
                .map_err(ort_err)
        };
        let session = if want_coreml {
            match Session::builder()
                .and_then(|b| {
                    b.with_execution_providers([CoreMLExecutionProvider::default().build()])
                })
                .and_then(|b| b.commit_from_file(path))
            {
                Ok(session) => session,
                Err(_) => cpu_session()?, // CoreML couldn't build → CPU
            }
        } else {
            cpu_session()?
        };
        Ok(Self {
            session,
            input: contract.input,
            output: contract.output,
            target_row,
            segment: contract.segment.max(1),
        })
    }

    /// Separate one segment. `planar` is `[2 × segment]` channel-major (all of
    /// L, then all of R). Returns this specialist's source as `[2 × segment]`
    /// (channel-major), extracted from the model's `[4, 2, segment]` output.
    pub fn run_segment(&mut self, planar: &[f32]) -> Result<Vec<f32>, StemError> {
        debug_assert_eq!(planar.len(), 2 * self.segment);
        let tensor = Tensor::from_array(([1usize, 2, self.segment], planar.to_vec()))
            .map_err(ort_err)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => tensor])
            .map_err(ort_err)?;
        // `[1, 4, 2, segment]` → contiguous C-order `[4, 2, segment]`.
        let (_, data) = outputs[self.output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(ort_err)?;
        let row = self.target_row * 2 * self.segment;
        let end = row + 2 * self.segment;
        if end > data.len() {
            return Err(StemError::Engine(format!(
                "model output too small: {} < {end}",
                data.len()
            )));
        }
        Ok(data[row..end].to_vec())
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
