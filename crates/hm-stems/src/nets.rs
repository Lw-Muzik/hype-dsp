//! The four Spleeter nets on ONNX Runtime — the only file that touches `ort`.
//!
//! The fp16 graphs (Best-Practice/spleeter-4stems-onnx) take magnitudes and
//! return magnitudes: input `x` and output `y`, both float32
//! `[2, num_splits, 512, 1024]`. ORT runs fp16 kernels where the CPU has them
//! and otherwise inserts casts to fp32, so the same files work everywhere.

use std::path::Path;

use ort::execution_providers::CPUExecutionProvider;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;

use crate::chunk::{NetRunner, TENSOR_LEN};
use crate::config::{NET_BINS, NET_COUNT, SPLIT_FRAMES};
use crate::StemError;

/// File name of each net inside the model directory, indexed by `NET_*`.
pub const NET_FILES: [&str; NET_COUNT] = ["vocals.onnx", "drums.onnx", "bass.onnx", "other.onnx"];

pub struct OrtNets {
    sessions: Vec<Session>,
}

/// Intra-op threads: leave a core for the audio and UI threads, cap at 4 (more
/// buys little on these small graphs and starves playback on weak CPUs).
fn intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(1)
        .clamp(1, 4)
}

impl OrtNets {
    /// Load the four nets from `dir`. Session options are what the mobile
    /// engine measured: graph optimisation EXTENDED (ALL keeps a second,
    /// re-laid-out copy of every weight), no CPU memory arena and no memory
    /// pattern (four arenas each held at their peak) — about 400 MB resident
    /// instead of ~1 GB, at the same speed.
    pub fn load(dir: &Path) -> Result<Self, StemError> {
        catch_ort(|| Self::load_inner(dir))
    }

    fn load_inner(dir: &Path) -> Result<Self, StemError> {
        let threads = intra_threads();
        let mut sessions = Vec::with_capacity(NET_COUNT);
        for file in NET_FILES {
            let session = Session::builder()
                .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level2))
                .and_then(|b| b.with_intra_threads(threads))
                .and_then(|b| b.with_inter_threads(1))
                .and_then(|b| b.with_memory_pattern(false))
                .and_then(|b| b.with_config_entry("session.intra_op.allow_spinning", "0"))
                .and_then(|b| {
                    b.with_execution_providers([CPUExecutionProvider::default()
                        .with_arena_allocator(false)
                        .build()])
                })
                .and_then(|b| b.commit_from_file(dir.join(file)))
                .map_err(ort_err)?;
            sessions.push(session);
        }
        Ok(Self { sessions })
    }
}

impl NetRunner for OrtNets {
    fn run(&mut self, net: usize, mag: &[f32], out: &mut [f32]) -> Result<(), StemError> {
        debug_assert_eq!(mag.len(), TENSOR_LEN);
        let session = self
            .sessions
            .get_mut(net)
            .ok_or_else(|| StemError::Engine(format!("no net {net}")))?;
        let input = Tensor::from_array(([2usize, 1, SPLIT_FRAMES, NET_BINS], mag.to_vec())).map_err(ort_err)?;
        let outputs = session.run(ort::inputs!["x" => input]).map_err(ort_err)?;
        let (_, data) = outputs["y"].try_extract_tensor::<f32>().map_err(ort_err)?;
        if data.len() != out.len() {
            return Err(StemError::Engine(format!(
                "net {net} returned {} values, expected {}",
                data.len(),
                out.len()
            )));
        }
        out.copy_from_slice(data);
        Ok(())
    }
}

/// Run `f`, turning a panic inside `ort` into an error the UI can show. With
/// `load-dynamic`, `ort` panics when it can't load the runtime library; keep
/// its message, which names the path and the loader's reason.
pub(crate) fn catch_ort<T>(
    f: impl FnOnce() -> Result<T, StemError> + std::panic::UnwindSafe,
) -> Result<T, StemError> {
    std::panic::catch_unwind(f).unwrap_or_else(|payload| {
        let why = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown error");
        Err(StemError::Engine(format!("the ONNX Runtime library could not be loaded: {why}")))
    })
}

fn ort_err(e: impl std::fmt::Display) -> StemError {
    StemError::Engine(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_inside_ort_becomes_an_error_that_keeps_the_reason() {
        let r: Result<(), StemError> = catch_ort(|| panic!("dlopen(/gone/libonnxruntime.dylib): no such file"));
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("could not be loaded"), "{msg}");
        assert!(msg.contains("/gone/libonnxruntime.dylib): no such file"), "{msg}");
        assert_eq!(catch_ort(|| Ok(7)).unwrap(), 7);
    }
}
