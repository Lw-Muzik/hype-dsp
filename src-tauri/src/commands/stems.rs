//! Live stem commands: **Vocals / Drums / Instruments** (Spleeter, MIT).
//!
//! Arming a track decodes it, separates the stretch around the playhead first,
//! swaps the stem mixer in at the live playhead as soon as that is ready, and
//! keeps separating the rest on a worker thread (cached per track once whole).
//! Where a part isn't separated yet the original plays, so the swap is gapless
//! and the controls simply take effect region by region. See `hm_stems` and
//! `hm_audio::stems`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hm_audio::{decode_bytes, decode_file, resample_stereo, AudioEngine, ELEMENT_COUNT};
use hm_core::IpcError;
use hm_stems::config::{chunk_containing, MODEL_RATE as RATE};
use hm_stems::download::{self, spleeter_manifest};
use hm_stems::{cache, runtime, Separator, StemError, StemStore};
use hm_ytmusic::YtMusicState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// How far past the playhead is separated before the stems swap in.
const LEAD_SECS: f64 = 30.0;

/// Managed state: the separator plus the bookkeeping for the running session
/// and in-app install.
pub struct StemState {
    pub separator: Arc<Separator>,
    /// `<app data>/stems` — models, runtime library and cache live below it.
    root: PathBuf,
    session: Mutex<Option<Session>>,
    install_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

/// The separation currently running and the flag that stops it.
struct Session {
    cancel: Arc<AtomicBool>,
}

impl StemState {
    pub fn new(root: PathBuf) -> Self {
        let manifest = spleeter_manifest();
        let separator = Separator::new(
            download::model_dir(&root.join("models"), &manifest),
            root.join("cache-spleeter-v1"),
        )
        .with_runtime_root(root.clone());
        Self {
            separator: Arc::new(separator),
            root,
            session: Mutex::new(None),
            install_cancel: Mutex::new(None),
        }
    }

    /// Drop everything the previous stems engine left behind (about 1.3 GB of
    /// models plus their caches) — those weights were research-only licensed.
    pub fn remove_legacy_stems_data(&self) {
        let _ = std::fs::remove_dir_all(self.root.join("cache"));
        let _ = std::fs::remove_dir_all(self.root.join("tmp"));
        // The old sidecar launcher script (`hm-<engine>`), if any.
        if let Ok(rd) = std::fs::read_dir(&self.root) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("hm-") && e.path().is_file() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let _ = std::fs::remove_dir_all(self.root.join("model"));
        // The runtime used to sit directly in the stems folder.
        if let Some(p) = runtime::lib_path(&self.root) {
            if let Some(name) = p.file_name() {
                let old = self.root.join(name);
                if old.is_file() && !p.is_file() {
                    let _ = std::fs::create_dir_all(self.root.join("runtime"));
                    let _ = std::fs::rename(&old, &p);
                }
            }
        }
    }

    fn models_root(&self) -> PathBuf {
        self.root.join("models")
    }

    fn cancel_session(&self) {
        if let Some(s) = self.session.lock().expect("stems session poisoned").take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Start a session, cancelling whatever ran before.
    fn begin(&self) -> Arc<AtomicBool> {
        let mut guard = self.session.lock().expect("stems session poisoned");
        if let Some(prev) = guard.take() {
            prev.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *guard = Some(Session { cancel: cancel.clone() });
        cancel
    }
}

/// What to separate: a file on disk, or a YouTube Music track.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StemSource {
    #[serde(rename_all = "camelCase")]
    File { path: String },
    #[serde(rename_all = "camelCase")]
    Ytmusic { video_id: String },
}

impl StemSource {
    fn key(&self) -> String {
        match self {
            StemSource::File { path } => cache::file_key(Path::new(path)),
            StemSource::Ytmusic { video_id } => cache::id_key("yt", video_id),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StemStatus {
    /// Models and runtime are installed: separation can run.
    available: bool,
    /// Bytes the in-app install would still download (0 when installed).
    download_bytes: u64,
    /// This track was separated before; arming is instant.
    separated: bool,
}

fn models_missing_bytes(state: &StemState) -> u64 {
    if state.separator.available() {
        0
    } else {
        spleeter_manifest().total_bytes()
    }
}

#[tauri::command]
pub fn stems_status(stems: State<'_, StemState>, source: Option<StemSource>) -> StemStatus {
    let download_bytes = models_missing_bytes(&stems) + runtime::download_bytes(&stems.root);
    StemStatus {
        available: download_bytes == 0,
        download_bytes,
        separated: source.is_some_and(|s| stems.separator.is_cached(&s.key())),
    }
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    done: u64,
    total: u64,
}

/// Download the ONNX Runtime and the four nets (hash-verified) into the app
/// data folder, emitting `stems:download` `{done,total}`. Safe to call when
/// already installed.
#[tauri::command(async)]
pub fn stems_install(app: AppHandle, stems: State<'_, StemState>) -> Result<(), IpcError> {
    let cancel = Arc::new(AtomicBool::new(false));
    *stems.install_cancel.lock().expect("stems install poisoned") = Some(cancel.clone());
    let rt_bytes = runtime::download_bytes(&stems.root);
    let manifest = spleeter_manifest();
    let model_bytes = if stems.separator.available() { 0 } else { manifest.total_bytes() };
    let total = rt_bytes + model_bytes;
    let emit = |done: u64| {
        let _ = app.emit("stems:download", DownloadProgress { done, total });
    };
    emit(0);
    let result = (|| {
        if rt_bytes > 0 {
            runtime::install(&stems.root, &|d, _| emit(d), &cancel)?;
        }
        if model_bytes > 0 {
            download::download(&stems.models_root(), &manifest, &|d, _| emit(rt_bytes + d), &cancel)?;
        }
        Ok::<(), StemError>(())
    })();
    *stems.install_cancel.lock().expect("stems install poisoned") = None;
    match result {
        Ok(()) => {
            emit(total);
            Ok(())
        }
        Err(StemError::Aborted) => Ok(()),
        Err(e) => Err(IpcError::new("stems", e.to_string())),
    }
}

/// Stop a running [`stems_install`].
#[tauri::command]
pub fn stems_install_cancel(stems: State<'_, StemState>) {
    if let Some(c) = stems.install_cancel.lock().expect("stems install poisoned").as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

/// Download a stream with 256 KB range requests. googlevideo throttles one
/// plain GET to a crawl but serves ranged ones at full speed.
fn fetch_ranged(url: &str, headers: &[(String, String)], cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    const RANGE: u64 = 256 * 1024;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut out: Vec<u8> = Vec::new();
    let mut total: Option<u64> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let start = out.len() as u64;
        let mut req = client.get(url).header("Range", format!("bytes={start}-{}", start + RANGE - 1));
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
        if total.is_none() {
            total = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|n| n.parse().ok());
        }
        let status = resp.status().as_u16();
        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        if status == 200 {
            // The server ignored Range and sent everything.
            return Ok(bytes.to_vec());
        }
        if bytes.is_empty() {
            break;
        }
        out.extend_from_slice(&bytes);
        if total.is_some_and(|t| out.len() as u64 >= t) {
            break;
        }
    }
    Ok(out)
}

/// Decode the source to interleaved stereo at 44.1 kHz.
fn load_mix(
    source: &StemSource,
    yt: &YtMusicState,
    cancel: &AtomicBool,
) -> Result<Vec<f32>, String> {
    let decoded = match source {
        StemSource::File { path } => decode_file(Path::new(path)).map_err(|e| e.to_string())?,
        StemSource::Ytmusic { video_id } => {
            let (url, headers) = yt.stream_target(video_id)?;
            let bytes = fetch_ranged(&url, &headers, cancel)?;
            decode_bytes(bytes, Some("m4a")).map_err(|e| e.to_string())?
        }
    };
    Ok(resample_stereo(&decoded.samples, decoded.sample_rate, RATE))
}

/// Arm stems for `source`: decode, separate around the playhead, swap in at the
/// live playhead. Returns once the stems are live (the rest keeps separating in
/// the background); emits `stems:progress` (0..1, share of the track separated).
#[tauri::command(async)]
pub fn stems_arm(
    app: AppHandle,
    stems: State<'_, StemState>,
    engine: State<'_, AudioEngine>,
    yt: State<'_, YtMusicState>,
    source: StemSource,
) -> Result<(), IpcError> {
    let key = source.key();
    let cancel = stems.begin();
    let fail = |m: String| IpcError::new("stems", m);

    let _ = app.emit("stems:progress", 0.0_f32);
    let mix = match load_mix(&source, &yt, &cancel) {
        Ok(m) => m,
        Err(_) if cancel.load(Ordering::Relaxed) => return Ok(()),
        Err(e) => return Err(fail(e)),
    };
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let store = StemStore::new(mix);
    if store.frames() == 0 {
        return Err(fail("this track has no audio to separate".into()));
    }

    let cached = stems.separator.load_cached(&key, &store);
    let playhead = || engine.pos().position_secs();
    store.set_wanted((playhead() * f64::from(RATE)) as usize);

    if !cached {
        if !stems.separator.available() {
            return Err(fail(StemError::Unavailable.to_string()));
        }
        let separator = stems.separator.clone();
        let worker_store = store.clone();
        let worker_cancel = cancel.clone();
        let progress_app = app.clone();
        let worker_key = key.clone();
        let worker = std::thread::Builder::new()
            .name("hm-stems".into())
            .spawn(move || {
                let result = separator.fill(&worker_store, &worker_cancel, |ready, total| {
                    let _ = progress_app.emit("stems:progress", ready as f32 / total.max(1) as f32);
                });
                match result {
                    Ok(()) => {
                        separator.save_cached(&worker_key, &worker_store);
                        let _ = progress_app.emit("stems:progress", 1.0_f32);
                        Ok(())
                    }
                    Err(StemError::Aborted) => {
                        separator.unload();
                        Ok(())
                    }
                    Err(e) => {
                        let message = e.to_string();
                        let _ = progress_app.emit("stems:error", message.clone());
                        Err(message)
                    }
                }
            })
            .map_err(|e| fail(e.to_string()))?;

        match wait_for_lead(&store, &cancel, &playhead, worker) {
            Ok(LeadWait::Ready) => {}
            Ok(LeadWait::Cancelled) => return Ok(()),
            Err(e) => return Err(fail(e)),
        }
    } else {
        let _ = app.emit("stems:progress", 1.0_f32);
    }

    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    // Swap in at the live playhead; unity gains reproduce the original mix.
    engine.play_stems(store, playhead()).map_err(|e| fail(e.to_string()))?;
    Ok(())
}

/// How the wait for the stretch around the playhead ended.
#[derive(Debug, PartialEq)]
enum LeadWait {
    /// It is separated (or the whole track is): swap the stems in.
    Ready,
    /// The session was cancelled (another track armed, or the view was left).
    Cancelled,
}

/// Wait until the playhead through `LEAD_SECS` past it is separated. `Err` when
/// the worker stops first — it failed (the message it also emitted on
/// `stems:error`) or panicked — so the panel stops waiting instead of spinning
/// under the error forever.
fn wait_for_lead(
    store: &StemStore,
    cancel: &AtomicBool,
    playhead: impl Fn() -> f64,
    worker: std::thread::JoinHandle<Result<(), String>>,
) -> Result<LeadWait, String> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(LeadWait::Cancelled);
        }
        // Read before checking readiness: a worker that finished cleanly had
        // already completed the store, so the check below sees that.
        let finished = worker.is_finished();
        let here = (playhead() * f64::from(RATE)) as usize;
        let last = ((here + (LEAD_SECS * f64::from(RATE)) as usize).min(store.frames() - 1)).max(here.min(store.frames() - 1));
        let (from, to) = (chunk_containing(here.min(store.frames() - 1)), chunk_containing(last));
        if (from..=to).all(|c| store.is_ready(c)) || store.is_complete() {
            return Ok(LeadWait::Ready);
        }
        if finished {
            return match worker.join() {
                Ok(Err(e)) => Err(e),
                // Stopped early without an error: it was cancelled.
                Ok(Ok(())) => Ok(LeadWait::Cancelled),
                Err(_) => Err("stem separation stopped unexpectedly".into()),
            };
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Set a control's gain live (0 = muted, 1 = unity). 0 vocals, 1 drums,
/// 2 instruments.
#[tauri::command]
pub fn stems_set_gain(engine: State<'_, AudioEngine>, stem: usize, gain: f32) {
    engine.set_stem_gain(stem, gain);
}

/// Back to the original mix (leaving the Stems view): every control to unity,
/// and any separation still running stops and frees its memory.
#[tauri::command]
pub fn stems_reset(engine: State<'_, AudioEngine>, stems: State<'_, StemState>) {
    stems.cancel_session();
    for e in 0..ELEMENT_COUNT {
        engine.set_stem_gain(e, 1.0);
    }
}

/// Current gains (vocals, drums, instruments).
#[tauri::command]
pub fn stems_gains(engine: State<'_, AudioEngine>) -> Vec<f32> {
    let gains = engine.stem_gains();
    (0..ELEMENT_COUNT).map(|i| gains.get(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-chunk track: the lead window is that chunk.
    fn short_store() -> Arc<StemStore> {
        StemStore::new(vec![0.0; 4000])
    }

    fn worker(f: impl FnOnce() -> Result<(), String> + Send + 'static) -> std::thread::JoinHandle<Result<(), String>> {
        std::thread::spawn(f)
    }

    #[test]
    fn a_failed_worker_ends_the_wait_with_its_error() {
        let store = short_store();
        let r = wait_for_lead(&store, &AtomicBool::new(false), || 0.0, worker(|| Err("stem engine error: boom".into())));
        assert_eq!(r, Err("stem engine error: boom".to_string()));
    }

    #[test]
    fn a_panicked_worker_ends_the_wait() {
        let store = short_store();
        let r = wait_for_lead(&store, &AtomicBool::new(false), || 0.0, worker(|| panic!("worker died")));
        assert!(r.unwrap_err().contains("unexpectedly"));
    }

    #[test]
    fn a_worker_that_stops_cleanly_early_was_cancelled() {
        let store = short_store();
        let r = wait_for_lead(&store, &AtomicBool::new(false), || 0.0, worker(|| Ok(())));
        assert_eq!(r, Ok(LeadWait::Cancelled));
    }

    #[test]
    fn cancelling_ends_the_wait() {
        let store = short_store();
        let r = wait_for_lead(&store, &AtomicBool::new(true), || 0.0, worker(|| Ok(())));
        assert_eq!(r, Ok(LeadWait::Cancelled));
    }

    #[test]
    fn a_separated_window_is_ready_even_while_the_worker_runs() {
        let store = short_store();
        let n = store.mix().len();
        assert!(store.fill_from(&vec![0.0; n], &vec![0.0; n]));
        let r = wait_for_lead(&store, &AtomicBool::new(false), || 0.0, worker(|| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }));
        assert_eq!(r, Ok(LeadWait::Ready));
    }

    #[test]
    fn a_worker_that_finishes_the_track_is_ready() {
        let store = short_store();
        let s = store.clone();
        let r = wait_for_lead(&store, &AtomicBool::new(false), || 0.0, worker(move || {
            let n = s.mix().len();
            assert!(s.fill_from(&vec![0.0; n], &vec![0.0; n]));
            Ok(())
        }));
        assert_eq!(r, Ok(LeadWait::Ready));
    }
}
