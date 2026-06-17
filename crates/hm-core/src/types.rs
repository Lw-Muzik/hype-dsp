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
    pub output: OutputState,
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
            output: OutputState::default(),
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineFrame {
    pub meters: MeterFrame,
    /// FFT magnitude bins (dB), when available this frame.
    pub spectrum: Option<Vec<f32>>,
}
