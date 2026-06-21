//! Stem-separation commands. Separates a track into stems via the Demucs sidecar
//! (offline, cached), then plays them through the engine where the UI's faders
//! mix them live. 4-stem (vocals/drums/bass/other) or 2-stem (vocals/instrumental,
//! ~2× faster). See `hm_stems` + `hm_audio::stems`.

use std::path::{Path, PathBuf};

use hm_audio::{decode_file, AudioEngine, DecodedAudio, STEM_COUNT};
use hm_core::IpcError;
use hm_stems::{Demucs, StemMode, Stems};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Managed state: the configured separator + a scratch dir for the input WAV.
pub struct StemState {
    pub demucs: Demucs,
    pub temp_dir: PathBuf,
}

#[derive(Serialize)]
pub struct StemStatus {
    /// The separator (sidecar + model) is installed.
    available: bool,
    /// This track has already been separated in this mode (cached).
    separated: bool,
}

#[tauri::command]
pub fn stems_status(
    stems: State<'_, StemState>,
    track_path: String,
    mode: StemMode,
) -> StemStatus {
    StemStatus {
        available: stems.demucs.available(),
        separated: stems.demucs.cached(Path::new(&track_path), mode).is_some(),
    }
}

/// Separate `track_path` for `mode` (using the cache if present) and start stem
/// playback. Emits `stems:progress` (0.0..=1.0) while separating.
#[tauri::command(async)]
pub fn stems_separate(
    app: AppHandle,
    stems: State<'_, StemState>,
    engine: State<'_, AudioEngine>,
    track_path: String,
    mode: StemMode,
) -> Result<(), IpcError> {
    let track = Path::new(&track_path);

    let result: Stems = if let Some(cached) = stems.demucs.cached(track, mode) {
        let _ = app.emit("stems:progress", 1.0_f32);
        cached
    } else {
        let decoded = decode_file(track).map_err(|e| IpcError::new("stems", e.to_string()))?;
        let input_wav = stems.temp_dir.join("stem_input.wav");
        write_wav_f32(&input_wav, &decoded.samples, decoded.sample_rate)
            .map_err(|e| IpcError::new("stems", e))?;
        let progress_app = app.clone();
        stems
            .demucs
            .separate(&input_wav, mode, move |p| {
                let _ = progress_app.emit("stems:progress", p);
            })
            .map_err(|e| IpcError::new("stems", e.to_string()))?
    };

    // Decode each stem into its playback slot; empty slots play as silence.
    let mut decoded: Vec<(usize, DecodedAudio)> = Vec::new();
    for sp in &result.stems {
        let d = decode_file(&sp.path).map_err(|e| IpcError::new("stems", e.to_string()))?;
        decoded.push((sp.slot, d));
    }
    let Some((_, first)) = decoded.first() else {
        return Err(IpcError::new("stems", "no stems produced".to_string()));
    };
    let rate = first.sample_rate;
    let meta = first.meta.clone();
    let mut buffers: [DecodedAudio; STEM_COUNT] = std::array::from_fn(|_| DecodedAudio {
        samples: Vec::new(),
        sample_rate: rate,
        meta: meta.clone(),
    });
    for (slot, d) in decoded {
        if slot < STEM_COUNT {
            buffers[slot] = d;
        }
    }

    engine
        .play_stems(buffers)
        .map_err(|e| IpcError::new("stems", e.to_string()))?;
    Ok(())
}

/// Set a stem's gain live (0 = muted, 1 = unity). Slots: 0 vocals, 1 drums/
/// instrumental, 2 bass, 3 other.
#[tauri::command]
pub fn stems_set_gain(engine: State<'_, AudioEngine>, stem: usize, gain: f32) {
    engine.set_stem_gain(stem, gain);
}

/// Current per-stem gains.
#[tauri::command]
pub fn stems_gains(engine: State<'_, AudioEngine>) -> Vec<f32> {
    let gains = engine.stem_gains();
    (0..STEM_COUNT).map(|i| gains.get(i)).collect()
}

/// Write interleaved-stereo float samples to a 32-bit float WAV.
fn write_wav_f32(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &s in samples {
        writer.write_sample(s).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}
