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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
pub struct BassBoostState {
    pub enabled: bool,
    /// Boost amount in dB applied by the low shelf.
    pub amount: f32,
    /// Adds gentle even-harmonic content to imply low end on small speakers.
    pub harmonics: bool,
    /// When true, scales boost down when the low-band energy is already strong
    /// (anti-overload / anti-mud). `false` = today's static-boost behavior.
    pub adaptive: bool,
}

impl Default for BassBoostState {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 0.0,
            harmonics: false,
            adaptive: false,
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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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

/// User LiveProg script (EEL2-subset). The compiled program lives engine-side;
/// only the source text is part of the serializable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptState {
    pub enabled: bool,
    pub source: String,
}

/// Makeup gain followed by the look-ahead brickwall limiter that keeps boosted
/// volume from clipping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
pub struct PlaybackState {
    /// Play a track list with no silence between tracks.
    pub gapless: bool,
    /// Crossfade duration in seconds (0 = off). Implies gapless when > 0.
    pub crossfade_secs: f32,
    /// Low-bandwidth mode: stream progressively (no full-download / prefetch),
    /// bigger buffers. Forces progressive single-track playback for cloud/phone.
    pub data_saver: bool,
    /// Keep the music going: a YT Music queue that runs out extends itself
    /// with the song radio of its last track. Read by the frontend only;
    /// stored here so the choice survives restarts. Defaults on — saved
    /// states from before the field existed get the new behaviour.
    #[serde(default = "default_autoplay")]
    pub autoplay: bool,
}

fn default_autoplay() -> bool {
    true
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            gapless: true,
            crossfade_secs: 0.0,
            data_saver: false,
            autoplay: true,
        }
    }
}

/// Per-headphone correction state: a preamp plus parametric bands loaded from
/// the active [`HeadphoneProfile`]. Empty/disabled when no profile is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
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
    pub script: ScriptState,
    pub headphone: HeadphoneCorrectionState,
    pub output: OutputState,
    pub playback: PlaybackState,
    /// Which apps the system-wide EQ tap processes (macOS). Default = all apps.
    pub system_eq_scope: SystemEqScope,
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
            script: ScriptState::default(),
            headphone: HeadphoneCorrectionState::default(),
            output: OutputState::default(),
            playback: PlaybackState::default(),
            system_eq_scope: SystemEqScope::default(),
            active_preset_id: None,
            active_profile_id: None,
        }
    }
}

/// How the macOS system-wide EQ tap selects which apps to process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemEqScopeMode {
    /// Process the whole system (every app except HypeMuzik). The default.
    #[default]
    All,
    /// Process only the listed apps (allowlist).
    Only,
    /// Process every app except the listed ones (blocklist).
    Except,
}

/// Per-app selection for the system-wide EQ. `apps` holds the stable session
/// ids reported by the mixer (`AppSession.id`), resolved to Core Audio process
/// objects when the tap is (re)built. Ignored unless system-wide EQ is active.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemEqScope {
    pub mode: SystemEqScopeMode,
    pub apps: Vec<String>,
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

/// One page of library tracks. `tracks` is the available subset (rows whose
/// file is currently reachable); `scanned` is the number of DB rows the page
/// read **before** availability filtering, so the caller can advance its offset
/// correctly even when some rows are hidden (a disconnected drive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub tracks: Vec<LibraryTrack>,
    pub scanned: i64,
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

/// A television channel from the world TV directory (iptv-org). Unlike a
/// [`RadioStation`], the `url` is a video stream (usually HLS) played by the
/// native mpv window, and `user_agent`/`referrer` carry the HTTP headers some
/// streams require.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvChannel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub logo: Option<String>,
    /// Category / genre from the playlist's `group-title` (e.g. "News").
    pub group: Option<String>,
    /// ISO 3166-1 alpha-2 country code, when known.
    pub country: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    /// Resolution hint parsed from the channel name (e.g. "720p"), when present.
    pub quality: Option<String>,
}

/// A browsable TV category (iptv-org `group-title` bucket).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvCategory {
    /// The iptv-org category id used in the playlist URL (e.g. "news").
    pub id: String,
    /// Display name (e.g. "News").
    pub name: String,
}

/// A country in the world TV browser (ISO 3166-1 alpha-2 code + English name).
/// The frontend renders the flag from the code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvCountry {
    pub code: String,
    pub name: String,
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
    fn bass_boost_default_adaptive_is_false() {
        let b = BassBoostState::default();
        assert!(!b.adaptive);
        assert!(!EngineState::default().bass.adaptive);
    }

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

    #[test]
    fn engine_state_deserializes_with_missing_fields() {
        // A partial blob (as if saved before newer stages existed) must load,
        // filling the absent fields from Default — not error.
        let json = r#"{"power":true,"masterVolume":1.0,"eq":{"enabled":true,"preGain":0.0,"bands":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}"#;
        let st: EngineState = serde_json::from_str(json).expect("partial EngineState must deserialize");
        assert!(st.power);
        assert!(!st.saturation.enabled);   // absent → Default (disabled)
        assert!(!st.compander.enabled);     // absent → Default
        assert!(!st.convolver.enabled);     // absent → Default
        // Newest field absent → defaults to whole-system scope, no apps.
        assert_eq!(st.system_eq_scope.mode, SystemEqScopeMode::All);
        assert!(st.system_eq_scope.apps.is_empty());
    }

    #[test]
    fn system_eq_scope_serde_roundtrip() {
        let scope = SystemEqScope {
            mode: SystemEqScopeMode::Only,
            apps: vec!["com.spotify.client".into(), "com.apple.Safari".into()],
        };
        let json = serde_json::to_string(&scope).expect("serialize");
        assert!(json.contains("\"only\""), "mode must serialize camelCase: {json}");
        let back: SystemEqScope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(scope, back);
    }

    #[test]
    fn engine_state_full_roundtrip() {
        // Serialize a full EngineState::default() to JSON, deserialize, assert equal.
        let original = EngineState::default();
        let json = serde_json::to_string(&original).expect("serialize must succeed");
        let restored: EngineState = serde_json::from_str(&json).expect("deserialize must succeed");
        assert_eq!(original, restored);
    }

    #[test]
    fn script_default_is_disabled_empty() {
        let s = ScriptState::default();
        assert!(!s.enabled);
        assert!(s.source.is_empty());
        assert!(!EngineState::default().script.enabled);
    }
}
