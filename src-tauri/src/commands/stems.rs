//! Stem-separation commands. Separates the **currently playing** track into four
//! stems in-process (htdemucs on CoreML, cached), then swaps them in at the live
//! playhead so the UI's faders mix them instantly — VirtualDJ-style.
//!
//! There's no manual "separate" step: the Stems view *arms* the current track
//! automatically, the track keeps playing while htdemucs runs (a few seconds on
//! Apple Silicon), then the stems swap in seamlessly (at unity gain they sum
//! back to the original mix). "2-stem" vs "4-stem" is a pure UI regrouping of
//! the same one-pass result, so switching modes is free. See `hm_stems` +
//! `hm_audio::stems`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use hm_audio::{decode_file, resample_stereo, AudioEngine, DecodedAudio, ELEMENT_COUNT, STEM_COUNT};
use hm_core::{IpcError, TrackMeta};
use hm_stems::{Separator, StemError, StemSet, STEM_SR};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Managed state: the configured htdemucs separator (model + on-disk cache).
pub struct StemState {
    pub separator: Separator,
}

/// The separation currently in flight, if any: which track it's for and the
/// flag that aborts it. Arming a *different* track (or leaving the Stems view
/// via [`stems_reset`]) trips the flag so an abandoned separation stops burning
/// CPU instead of grinding to completion. This is process-wide state like the
/// separator itself; it lives in a static because `StemState` is constructed
/// with a struct literal during app setup.
struct InFlight {
    track: String,
    cancel: Arc<AtomicBool>,
}

fn in_flight() -> &'static Mutex<Option<InFlight>> {
    static IN_FLIGHT: OnceLock<Mutex<Option<InFlight>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(None))
}

/// Register `track` as the armed separation, cancelling any in-flight run for a
/// *different* track. Re-arming the same track shares the existing flag so one
/// later cancel stops every pass for it.
fn arm_in_flight(track: &str) -> Arc<AtomicBool> {
    let mut guard = in_flight().lock().expect("stems in-flight poisoned");
    if let Some(prev) = guard.take() {
        if prev.track == track {
            let cancel = prev.cancel.clone();
            *guard = Some(prev);
            return cancel;
        }
        prev.cancel.store(true, Ordering::Relaxed);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *guard = Some(InFlight {
        track: track.to_owned(),
        cancel: cancel.clone(),
    });
    cancel
}

/// Clear the in-flight slot if it still belongs to `cancel`'s arm (a newer arm
/// may have replaced it already).
fn finish_in_flight(cancel: &Arc<AtomicBool>) {
    let mut guard = in_flight().lock().expect("stems in-flight poisoned");
    if guard
        .as_ref()
        .is_some_and(|f| Arc::ptr_eq(&f.cancel, cancel))
    {
        *guard = None;
    }
}

/// Cancel whatever separation is in flight (used on disarm/close).
fn cancel_in_flight() {
    if let Some(prev) = in_flight()
        .lock()
        .expect("stems in-flight poisoned")
        .take()
    {
        prev.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StemStatus {
    /// The model (and ONNX Runtime) are installed — separation can run.
    available: bool,
    /// This track is already separated (cached) — arming is instant.
    separated: bool,
    /// CoreML (Neural Engine / GPU) is driving inference, vs. the CPU fallback.
    accelerated: bool,
}

#[tauri::command]
pub fn stems_status(stems: State<'_, StemState>, track_path: String) -> StemStatus {
    let available = stems.separator.available();
    StemStatus {
        available,
        separated: stems.separator.cached(Path::new(&track_path)).is_some(),
        accelerated: available && stems.separator.accelerated(),
    }
}

/// Arm stems for `track_path`: separate it (using the cache if present) and swap
/// the stems in at the live playhead. Emits `stems:progress` (0.0..=1.0) while
/// htdemucs runs — the track keeps playing throughout.
#[tauri::command(async)]
pub fn stems_arm(
    app: AppHandle,
    stems: State<'_, StemState>,
    engine: State<'_, AudioEngine>,
    track_path: String,
) -> Result<(), IpcError> {
    let track = Path::new(&track_path);
    // Arming supersedes any separation still running for another track.
    let cancel = arm_in_flight(&track_path);

    let set: StemSet = if let Some(cached) = stems.separator.cached(track) {
        finish_in_flight(&cancel);
        let _ = app.emit("stems:progress", 1.0_f32);
        cached
    } else {
        let decoded = match decode_file(track) {
            Ok(d) => d,
            Err(e) => {
                finish_in_flight(&cancel);
                return Err(IpcError::new("stems", e.to_string()));
            }
        };
        // htdemucs runs at 44.1 kHz; resample the mixture up front.
        let mix = resample_stereo(&decoded.samples, decoded.sample_rate, STEM_SR);
        let progress_app = app.clone();
        let result = stems.separator.separate_cancellable(
            &mix,
            track,
            move |p| {
                let _ = progress_app.emit("stems:progress", p);
            },
            &cancel,
        );
        finish_in_flight(&cancel);
        match result {
            Ok(set) => set,
            // Superseded by a newer arm (or a disarm) — silently step aside.
            Err(StemError::Aborted) => return Ok(()),
            Err(e) => return Err(IpcError::new("stems", e.to_string())),
        }
    };

    let buffers: [DecodedAudio; STEM_COUNT] = set.map(|samples| DecodedAudio {
        samples,
        sample_rate: STEM_SR,
        meta: TrackMeta::default(),
    });

    // Swap in at the live playhead so the track continues without a gap.
    let start = engine.pos().position_secs();
    engine
        .play_stems(buffers, start)
        .map_err(|e| IpcError::new("stems", e.to_string()))?;
    Ok(())
}

/// Set an element's gain live (0 = muted, 1 = unity). Elements: 0 vocals,
/// 1 kick, 2 hihat, 3 bass, 4 melody.
#[tauri::command]
pub fn stems_set_gain(engine: State<'_, AudioEngine>, stem: usize, gain: f32) {
    engine.set_stem_gain(stem, gain);
}

/// Reset every element to unity so the mix sounds like the original track again
/// (used when leaving the Stems view).
#[tauri::command]
pub fn stems_reset(engine: State<'_, AudioEngine>) {
    // Leaving the view also abandons any separation still running — stop it
    // rather than let it grind on for a result nobody will hear.
    cancel_in_flight();
    for e in 0..ELEMENT_COUNT {
        engine.set_stem_gain(e, 1.0);
    }
}

/// Current per-element gains (vocals, kick, hihat, bass, melody).
#[tauri::command]
pub fn stems_gains(engine: State<'_, AudioEngine>) -> Vec<f32> {
    let gains = engine.stem_gains();
    (0..ELEMENT_COUNT).map(|i| gains.get(i)).collect()
}
