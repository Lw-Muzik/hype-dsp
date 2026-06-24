//! Canonical data model shared by the DSP engine, media subsystems, and the
//! Tauri/React front end.
//!
//! These types are the single source of truth for the app's configuration and
//! real-time telemetry. They are mirrored verbatim by TypeScript interfaces in
//! `src/lib/types.ts`; the `camelCase` rename keeps the JSON identical on both
//! sides.

use serde::{Deserialize, Serialize};

/// Number of graphic-EQ bands (ISO one-third-octave centers, 20 Hz–20 kHz).
pub const BAND_COUNT: usize = 31;

/// ISO one-third-octave nominal center frequencies, in Hz, for the 31 bands.
///
/// Owned here so the UI band labels and the DSP filter centers can never drift
/// apart — the engine and the front end both read from this one array.
pub const ISO_CENTERS_HZ: [f32; BAND_COUNT] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

/// State of the 31-band graphic equalizer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqState {
    pub enabled: bool,
    /// Pre-gain applied ahead of the band filters, in dB.
    pub pre_gain: f32,
    /// Per-band gain in dB; index maps to [`ISO_CENTERS_HZ`].
    pub bands: [f32; BAND_COUNT],
}

impl Default for EqState {
    fn default() -> Self {
        Self {
            enabled: true,
            pre_gain: 0.0,
            bands: [0.0; BAND_COUNT],
        }
    }
}

/// Low-shelf bass boost with an optional harmonic-enhancement toggle for small
/// drivers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BassBoostState {
    pub enabled: bool,
    /// Boost amount in dB applied by the low shelf.
    pub amount: f32,
    /// Adds gentle even-harmonic content to imply low end on small speakers.
    pub harmonics: bool,
}

impl Default for BassBoostState {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 0.0,
            harmonics: false,
        }
    }
}

/// Spatialization algorithm for out-of-head imaging on headphones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpatialMode {
    /// Crossfeed + stereo widening (the dependency-free baseline).
    Crossfeed,
    /// HRTF convolution against a loaded HRIR set (e.g. MIT KEMAR).
    Hrtf,
}

/// Virtual-surround / widening stage state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpatializerState {
    pub enabled: bool,
    /// 0.0 = none, 1.0 = maximum effect.
    pub amount: f32,
    pub mode: SpatialMode,
}

impl Default for SpatializerState {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 0.5,
            mode: SpatialMode::Crossfeed,
        }
    }
}

/// On/off state for each virtual loudspeaker in the 3D Surround ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurroundSpeakers {
    pub front_l: bool,
    pub front_r: bool,
    /// The ±90° side ("tweeter") pair.
    pub side_l: bool,
    pub side_r: bool,
    /// The ±135° rear pair.
    pub surround_l: bool,
    pub surround_r: bool,
}

impl Default for SurroundSpeakers {
    fn default() -> Self {
        Self {
            front_l: true,
            front_r: true,
            side_l: true,
            side_r: true,
            surround_l: true,
            surround_r: true,
        }
    }
}

/// Virtual-surround-over-headphones ("3D Surround") stage state: a ring of
/// virtual loudspeakers rendered binaurally. Distinct from [`SpatializerState`],
/// which is the lightweight crossfeed/HRTF widener.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Surround3DState {
    pub enabled: bool,
    /// Overall wet/dry of the virtual-surround effect (0.0 = dry … 1.0 = full).
    pub intensity: f32,
    /// LFE / subwoofer level (0.0 … 1.0).
    pub subwoofer: f32,
    pub speakers: SurroundSpeakers,
}

impl Default for Surround3DState {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 0.5,
            subwoofer: 0.25,
            speakers: SurroundSpeakers::default(),
        }
    }
}

/// Room reverb ("room effects") — a Freeverb-style algorithmic reverb, ported
/// from the Hype mobile app. All scalar params are 0.0–1.0 except `pre_delay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomState {
    pub enabled: bool,
    /// Scales the comb delay lengths (0 = tiny room … 1 = huge).
    pub room_size: f32,
    /// Reverb tail length (maps to comb feedback / RT60).
    pub decay: f32,
    /// High-frequency absorption (0 = bright … 1 = dark).
    pub damping: f32,
    /// Pre-delay before the reverb tail, in milliseconds (0–200).
    pub pre_delay: f32,
    /// Echo density via the allpass diffusers (0 = sparse … 1 = dense).
    pub diffusion: f32,
    /// Wet/dry mix (0 = dry … 1 = fully wet).
    pub wet_dry: f32,
    /// Active room-preset id, if one is applied (for the UI).
    pub active_preset_id: Option<String>,
}

impl Default for RoomState {
    fn default() -> Self {
        // "Medium Room" values, but off by default.
        Self {
            enabled: false,
            room_size: 0.4,
            decay: 0.4,
            damping: 0.45,
            pre_delay: 8.0,
            diffusion: 0.55,
            wet_dry: 0.3,
            active_preset_id: None,
        }
    }
}

/// Convolution (impulse-response) stage state. The heavy IR data is NOT stored
/// here — it is published to the audio stage out-of-band via a lock-free slot.
/// These are only the cheap scalars the audio thread reads each block, plus
/// metadata the UI displays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvolverState {
    pub enabled: bool,
    /// Wet/dry mix (0 = dry … 1 = fully wet). Correction IRs run ~1.0.
    pub wet_dry: f32,
    /// Post-convolution trim in dB applied to the wet path.
    pub ir_gain_db: f32,
    /// Identifier (path or bundled id) of the loaded IR, for the UI.
    pub ir_id: Option<String>,
    /// Human-facing IR name, for the UI.
    pub ir_name: Option<String>,
    /// IR length in seconds after the length cap.
    pub ir_seconds: f32,
    /// Whether the IR was truncated by the length cap.
    pub ir_truncated: bool,
}

impl Default for ConvolverState {
    fn default() -> Self {
        Self {
            enabled: false,
            wet_dry: 1.0,
            ir_gain_db: 0.0,
            ir_id: None,
            ir_name: None,
            ir_seconds: 0.0,
            ir_truncated: false,
        }
    }
}

/// Multiband compander (10-band Linkwitz-Riley compressor/expander). Global
/// params are applied uniformly to every band. Ported from the mobile Hype MBC.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanderState {
    pub enabled: bool,
    /// Compression starts above this input level, in dB.
    pub threshold_db: f32,
    /// Compression ratio (1.0 = no compression).
    pub ratio: f32,
    /// Soft-knee width in dB (0 = hard knee).
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Post (makeup) gain in dB.
    pub makeup_db: f32,
    /// Noise-gate threshold in dB; below it the expander engages.
    pub gate_db: f32,
    /// Expander ratio applied below the gate threshold (>= 1).
    pub expander_ratio: f32,
}

impl Default for CompanderState {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.5,
            knee_db: 8.0,
            attack_ms: 15.0,
            release_ms: 45.0,
            makeup_db: 0.0,
            gate_db: -70.0,
            expander_ratio: 2.0,
        }
    }
}

/// Tube-style analog saturation (4× oversampled, 2nd-harmonic warmth).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaturationState {
    pub enabled: bool,
    /// 0..1 → internal tanh drive amount.
    pub drive: f32,
    /// Dry/wet mix, 0..1.
    pub mix: f32,
}

impl Default for SaturationState {
    fn default() -> Self {
        Self {
            enabled: false,
            drive: 0.3,
            mix: 1.0,
        }
    }
}

/// Makeup gain followed by the look-ahead brickwall limiter that keeps boosted
/// volume from clipping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    /// Makeup gain in dB applied before the limiter.
    pub gain_db: f32,
    pub limiter_enabled: bool,
    /// Brickwall ceiling in dBFS the output must never exceed.
    pub ceiling_db: f32,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            limiter_enabled: true,
            ceiling_db: -0.3,
        }
    }
}

/// Queue-playback behaviour: gapless transitions and crossfade between tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    /// Play a track list with no silence between tracks.
    pub gapless: bool,
    /// Crossfade duration in seconds (0 = off). Implies gapless when > 0.
    pub crossfade_secs: f32,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            gapless: true,
            crossfade_secs: 0.0,
        }
    }
}

/// Per-headphone correction state: a preamp plus parametric bands loaded from
/// the active [`HeadphoneProfile`]. Empty/disabled when no profile is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadphoneCorrectionState {
    pub enabled: bool,
    pub preamp: f32,
    pub bands: Vec<ParametricBand>,
}

impl Default for HeadphoneCorrectionState {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp: 0.0,
            bands: Vec::new(),
        }
    }
}

/// The complete, serializable enhancement state mirrored by the Zustand store.
///
/// This is what `engine_get_state` returns and what every `engine_set_*`
/// command mutates. The audio thread never reads this directly — it reads an
/// immutable snapshot derived from it (see `hm-audio`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineState {
    /// Master bypass. When `false` the chain passes audio through untouched.
    pub power: bool,
    /// Master output volume as a linear gain (1.0 = unity; > 1.0 = boost).
    pub master_volume: f32,
    pub eq: EqState,
    pub bass: BassBoostState,
    pub spatializer: SpatializerState,
    pub surround3d: Surround3DState,
    pub room: RoomState,
    pub convolver: ConvolverState,
    pub compander: CompanderState,
    pub saturation: SaturationState,
    pub headphone: HeadphoneCorrectionState,
    pub output: OutputState,
    pub playback: PlaybackState,
    /// Active preset id, if one is applied.
    pub active_preset_id: Option<String>,
    /// Active headphone profile id, if one is applied.
    pub active_profile_id: Option<String>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            power: true,
            master_volume: 1.0,
            eq: EqState::default(),
            bass: BassBoostState::default(),
            spatializer: SpatializerState::default(),
            surround3d: Surround3DState::default(),
            room: RoomState::default(),
            convolver: ConvolverState::default(),
            compander: CompanderState::default(),
            saturation: SaturationState::default(),
            headphone: HeadphoneCorrectionState::default(),
            output: OutputState::default(),
            playback: PlaybackState::default(),
            active_preset_id: None,
            active_profile_id: None,
        }
    }
}

/// A named equalizer preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqPreset {
    pub id: String,
    pub name: String,
    /// `true` for shipped genre presets, `false` for user-created ones.
    pub builtin: bool,
    pub bands: [f32; BAND_COUNT],
    pub pre_gain: f32,
}

/// One parametric band of a headphone correction curve (AutoEq format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParametricBand {
    /// Filter type, e.g. `peaking`, `lowShelf`, `highShelf`.
    pub kind: String,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

/// Per-model headphone correction profile sourced from an AutoEq-format dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadphoneProfile {
    pub id: String,
    pub brand: String,
    pub model: String,
    /// Global preamp in dB recommended by the dataset.
    pub preamp: f32,
    pub bands: Vec<ParametricBand>,
}

/// A scanned local library track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTrack {
    /// Absolute file path (the track's identity).
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Genre from the file's tags, used for the library's category filter.
    pub genre: Option<String>,
    pub duration_secs: Option<f64>,
}

/// A user playlist (ordered list of track paths, stored separately).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
}

/// Now-playing metadata extracted from the decoded track's tags (ID3 / Vorbis
/// comments / MP4 atoms) and embedded cover art. Surfaced to the UI's docked
/// now-playing bar. The same path serves local files, cloud, and phone streams.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Embedded front-cover art as a `data:` URI (base64), if present.
    pub cover: Option<String>,
}

/// An internet radio station entry from the directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioStation {
    pub id: String,
    pub name: String,
    pub url: String,
    pub genre: Option<String>,
    pub country: Option<String>,
    pub favicon: Option<String>,
}

/// A single application's audio session, for the per-app mixer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSession {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// Linear volume, 0.0–1.0.
    pub volume: f32,
    pub muted: bool,
}

/// Per-channel peak and RMS levels for one processed block.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterFrame {
    /// Peak magnitude per channel `[left, right]`, linear 0.0–~1.0+.
    pub peak: [f32; 2],
    /// RMS magnitude per channel `[left, right]`, linear.
    pub rms: [f32; 2],
}

impl Default for MeterFrame {
    fn default() -> Self {
        Self {
            peak: [0.0; 2],
            rms: [0.0; 2],
        }
    }
}

/// One frame of real-time telemetry emitted to the UI over a Tauri channel.
///
/// `spectrum` is present only on the throttled cadence at which the FFT is
/// computed, so meter updates can run faster than the analyzer.
/// `compander_gr` carries per-band gain-reduction in dB (≤0) when playing,
/// or `None` when idle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineFrame {
    pub meters: MeterFrame,
    /// FFT magnitude bins (dB), when available this frame.
    pub spectrum: Option<Vec<f32>>,
    /// Per-band compander gain-reduction in dB (10 values, ≤0), or `None` when idle.
    pub compander_gr: Option<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convolver_default_is_disabled_and_empty() {
        let c = ConvolverState::default();
        assert!(!c.enabled);
        assert_eq!(c.wet_dry, 1.0);
        assert_eq!(c.ir_gain_db, 0.0);
        assert!(c.ir_id.is_none());
        assert!(!c.ir_truncated);
        // Present on EngineState and off by default.
        assert!(!EngineState::default().convolver.enabled);
    }

    #[test]
    fn compander_default_is_disabled_with_mastering_defaults() {
        let c = CompanderState::default();
        assert!(!c.enabled);
        assert_eq!(c.threshold_db, -18.0);
        assert_eq!(c.ratio, 2.5);
        assert_eq!(c.knee_db, 8.0);
        assert_eq!(c.attack_ms, 15.0);
        assert_eq!(c.release_ms, 45.0);
        assert_eq!(c.makeup_db, 0.0);
        assert_eq!(c.gate_db, -70.0);
        assert_eq!(c.expander_ratio, 2.0);
        assert!(!EngineState::default().compander.enabled);
    }

    #[test]
    fn saturation_default_is_disabled() {
        let s = SaturationState::default();
        assert!(!s.enabled);
        assert_eq!(s.drive, 0.3);
        assert_eq!(s.mix, 1.0);
        assert!(!EngineState::default().saturation.enabled);
    }
}
