//! Live stem separation for the player: **Vocals / Drums / Instruments**, with
//! Deezer's Spleeter 4-stem nets (code and pretrained models MIT) on ONNX
//! Runtime. A port of the HypeMuzik mobile engine (`hype/android/.../cpp/stems`),
//! checked against that C++ code by the `parity` test.
//!
//! Only vocals and drums are stored. The mix is
//! `out = gV·V + gD·D + gI·(M − V − D)`, so every gain at unity reproduces the
//! decoded track bit for bit, and the third control (instruments, i.e. bass +
//! other + residual) costs no memory.
//!
//! Work is split into fixed-grid chunks of ~11.8 s ([`config`]). A worker
//! separates the chunks around the playhead first and then fills in the rest in
//! any order the listener's position suggests, publishing each into a
//! [`StemStore`] the audio thread reads lock-free.

pub mod cache;
pub mod chunk;
pub mod config;
pub mod download;
pub mod mask;
mod nets;
pub mod runtime;
pub mod store;
pub mod stft;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub use chunk::{NetRunner, TENSOR_LEN};
pub use config::MODEL_RATE;
pub use nets::{OrtNets, NET_FILES};
pub use store::{Run, StemStore};

use chunk::ChunkScratch;

#[derive(Debug, thiserror::Error)]
pub enum StemError {
    #[error("the stem models aren't installed yet")]
    Unavailable,
    #[error("stem engine error: {0}")]
    Engine(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("download failed: {0}")]
    Network(String),
    /// Cancelled (the user armed a different track or left the Stems view) —
    /// not a failure; callers treat it as a silent no-op.
    #[error("stem separation aborted")]
    Aborted,
}

/// Separate chunks into `store` until it is complete (or `cancel` trips),
/// always picking the chunk nearest the listener first. `on_chunk` runs after
/// each chunk with `(ready, total)`. Generic over the nets so tests can use
/// stand-ins.
pub fn fill_store(
    runner: &mut dyn NetRunner,
    store: &StemStore,
    cancel: &AtomicBool,
    mut on_chunk: impl FnMut(usize, usize),
) -> Result<(), StemError> {
    let mut scratch = ChunkScratch::new();
    while let Some(c) = store.next_missing() {
        if cancel.load(Ordering::Relaxed) {
            return Err(StemError::Aborted);
        }
        let out = chunk::separate_chunk(runner, &mut scratch, store.mix(), c)?;
        if !store.publish(c, out) {
            return Err(StemError::Engine(format!("chunk {c} could not be stored")));
        }
        on_chunk(store.ready_count(), store.chunk_total());
    }
    Ok(())
}

/// The Spleeter separator: model directory, on-disk result cache, and the four
/// loaded ONNX sessions (loaded lazily, dropped by [`unload`](Self::unload)).
pub struct Separator {
    model_dir: PathBuf,
    cache_dir: PathBuf,
    /// Where [`runtime::install`] put the ONNX Runtime library, if the app
    /// manages it (otherwise `ORT_DYLIB_PATH` / the system search path decide).
    runtime_root: Option<PathBuf>,
    nets: Mutex<Option<OrtNets>>,
}

/// Cache budget before the oldest entries are evicted.
const CACHE_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

impl Separator {
    pub fn new(model_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self { model_dir, cache_dir, runtime_root: None, nets: Mutex::new(None) }
    }

    /// Use the ONNX Runtime library installed under `root` (see [`runtime`]).
    pub fn with_runtime_root(mut self, root: PathBuf) -> Self {
        self.runtime_root = Some(root);
        self
    }

    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Whether all four net files are present.
    pub fn available(&self) -> bool {
        NET_FILES.iter().all(|f| self.model_dir.join(f).is_file())
    }

    /// Has this track been fully separated before?
    pub fn is_cached(&self, key: &str) -> bool {
        cache::exists(&self.cache_dir, key)
    }

    /// Fill `store` from the cache. Returns false on a miss.
    pub fn load_cached(&self, key: &str, store: &StemStore) -> bool {
        cache::read(&self.cache_dir, key, store.frames())
            .is_some_and(|(v, d)| store.fill_from(&v, &d))
    }

    /// Cache a completed store (no-op if it isn't complete).
    pub fn save_cached(&self, key: &str, store: &StemStore) {
        let Some((v, d)) = store.assemble() else { return };
        if std::fs::create_dir_all(&self.cache_dir).is_ok() && cache::write(&self.cache_dir, key, &v, &d).is_ok() {
            cache::prune(&self.cache_dir, CACHE_BUDGET_BYTES);
        }
    }

    /// Separate `store` with the real nets. Blocking and CPU-heavy: run it off
    /// the audio and UI threads. Holds the net lock for the whole run, so a
    /// second separation waits for (or cancels) the first.
    pub fn fill(
        &self,
        store: &StemStore,
        cancel: &AtomicBool,
        on_chunk: impl FnMut(usize, usize),
    ) -> Result<(), StemError> {
        if !self.available() {
            return Err(StemError::Unavailable);
        }
        let mut guard = self
            .nets
            .lock()
            .map_err(|_| StemError::Engine("separator lock poisoned".into()))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(StemError::Aborted);
        }
        if guard.is_none() {
            if let Some(root) = &self.runtime_root {
                if !runtime::is_ready(root) {
                    return Err(StemError::Unavailable);
                }
                runtime::activate(root)?;
            }
            *guard = Some(OrtNets::load(&self.model_dir)?);
        }
        let nets = guard.as_mut().expect("just loaded");
        fill_store(nets, store, cancel, on_chunk)
    }

    /// Drop the loaded nets (~400 MB) if no separation is running.
    pub fn unload(&self) {
        if let Ok(mut guard) = self.nets.try_lock() {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::chunk::test_nets::ScaleNets;
    use super::*;

    #[test]
    fn unavailable_without_models() {
        let s = Separator::new("/no/such/models".into(), std::env::temp_dir());
        assert!(!s.available());
        let store = StemStore::new(vec![0.0; 2000]);
        assert!(matches!(s.fill(&store, &AtomicBool::new(false), |_, _| {}), Err(StemError::Unavailable)));
    }

    #[test]
    fn cancelled_fill_stops_before_doing_work() {
        let store = StemStore::new(vec![0.0; 2000]);
        let r = fill_store(&mut ScaleNets([1.0; 4]), &store, &AtomicBool::new(true), |_, _| {});
        assert!(matches!(r, Err(StemError::Aborted)));
        assert_eq!(store.ready_count(), 0);
    }

    #[test]
    fn fill_starts_at_the_playhead_and_completes_the_track() {
        let frames = config::STEP * 3 + 1000;
        let store = StemStore::new(vec![0.1; frames * 2]);
        store.set_wanted(config::owned_start(2) + 5);
        let order = std::cell::RefCell::new(Vec::new());
        fill_store(&mut ScaleNets([1.0; 4]), &store, &AtomicBool::new(false), |ready, total| {
            order.borrow_mut().push((ready, total));
        })
        .unwrap();
        assert!(store.is_complete());
        assert_eq!(order.borrow().last(), Some(&(store.chunk_total(), store.chunk_total())));
        // equal nets: V = D = mix/4 once everything is stitched.
        let (v, d) = store.assemble().unwrap();
        assert_eq!(v.len(), frames * 2);
        assert!(v.iter().zip(&d).all(|(a, b)| (a - 0.025).abs() < 1e-4 && (b - 0.025).abs() < 1e-4));
    }

    #[test]
    fn cache_round_trip_reproduces_the_store() {
        let dir = std::env::temp_dir().join(format!("hm_stems_sep_cache_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sep = Separator::new("/none".into(), dir.clone());
        let frames = 30_000;
        let mix: Vec<f32> = (0..frames * 2).map(|i| (i as f32 * 0.001).sin() * 0.5).collect();
        let a = StemStore::new(mix.clone());
        fill_store(&mut ScaleNets([2.0, 1.0, 1.0, 1.0]), &a, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(!sep.is_cached("k"));
        sep.save_cached("k", &a);
        assert!(sep.is_cached("k"));
        let b = StemStore::new(mix);
        assert!(sep.load_cached("k", &b));
        let (av, ad) = a.assemble().unwrap();
        let (bv, bd) = b.assemble().unwrap();
        assert!(av.iter().zip(&bv).chain(ad.iter().zip(&bd)).all(|(x, y)| (x - y).abs() < 1e-4));
        // a different-length decode can't use the entry
        assert!(!sep.load_cached("k", &StemStore::new(vec![0.0; 4000])));
        let _ = std::fs::remove_dir_all(dir);
    }
}
