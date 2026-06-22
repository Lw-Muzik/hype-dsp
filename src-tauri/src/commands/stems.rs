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

use hm_audio::{decode_file, resample_stereo, AudioEngine, DecodedAudio, ELEMENT_COUNT, STEM_COUNT};
use hm_core::{IpcError, TrackMeta};
use hm_stems::{Separator, StemSet, STEM_SR};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Managed state: the configured htdemucs separator (model + on-disk cache).
pub struct StemState {
    pub separator: Separator,
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

    let set: StemSet = if let Some(cached) = stems.separator.cached(track) {
        let _ = app.emit("stems:progress", 1.0_f32);
        cached
    } else {
        let decoded = decode_file(track).map_err(|e| IpcError::new("stems", e.to_string()))?;
        // htdemucs runs at 44.1 kHz; resample the mixture up front.
        let mix = resample_stereo(&decoded.samples, decoded.sample_rate, STEM_SR);
        let progress_app = app.clone();
        stems
            .separator
            .separate(&mix, track, move |p| {
                let _ = progress_app.emit("stems:progress", p);
            })
            .map_err(|e| IpcError::new("stems", e.to_string()))?
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
