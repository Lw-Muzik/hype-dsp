/**
 * Canonical front-end types.
 *
 * These mirror the serde payloads in `crates/hm-core` exactly — same field
 * names (camelCase), same shapes. They are one contract expressed twice; when a
 * Rust type changes, change it here too.
 */

/** Number of graphic-EQ bands (ISO one-third-octave, 20 Hz–20 kHz). */
export const BAND_COUNT = 31;

/** ISO one-third-octave nominal center frequencies (Hz), mirroring Rust. */
export const ISO_CENTERS_HZ: readonly number[] = [
  20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630,
  800, 1000, 1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500,
  16000, 20000,
];

export interface AppInfo {
  name: string;
  version: string;
  engineSchema: number;
}

export interface EqState {
  enabled: boolean;
  preGain: number;
  /** Per-band gain in dB; length === BAND_COUNT. */
  bands: number[];
}

export interface BassBoostState {
  enabled: boolean;
  amount: number;
  harmonics: boolean;
}

export type SpatialMode = "crossfeed" | "hrtf";

export interface SpatializerState {
  enabled: boolean;
  /** 0.0 = none, 1.0 = maximum. */
  amount: number;
  mode: SpatialMode;
}

export interface OutputState {
  gainDb: number;
  limiterEnabled: boolean;
  ceilingDb: number;
}

export interface HeadphoneCorrectionState {
  enabled: boolean;
  preamp: number;
  bands: ParametricBand[];
}

export interface EngineState {
  power: boolean;
  /** Linear master gain (1.0 = unity, > 1.0 = boost). */
  masterVolume: number;
  eq: EqState;
  bass: BassBoostState;
  spatializer: SpatializerState;
  headphone: HeadphoneCorrectionState;
  output: OutputState;
  activePresetId: string | null;
  activeProfileId: string | null;
}

export interface EqPreset {
  id: string;
  name: string;
  builtin: boolean;
  bands: number[];
  preGain: number;
}

export interface ParametricBand {
  kind: string;
  freq: number;
  gain: number;
  q: number;
}

export interface HeadphoneProfile {
  id: string;
  brand: string;
  model: string;
  preamp: number;
  bands: ParametricBand[];
}

export interface RadioStation {
  id: string;
  name: string;
  url: string;
  genre: string | null;
  country: string | null;
  favicon: string | null;
}

export interface AppSession {
  id: string;
  name: string;
  icon: string | null;
  /** Linear volume, 0.0–1.0. */
  volume: number;
  muted: boolean;
}

export type LicenseStatus =
  | { kind: "trial"; daysLeft: number }
  | { kind: "licensed" }
  | { kind: "expired" };

export interface MeterFrame {
  /** Peak magnitude per channel [left, right], linear. */
  peak: [number, number];
  /** RMS magnitude per channel [left, right], linear. */
  rms: [number, number];
}

export interface EngineFrame {
  meters: MeterFrame;
  /** FFT magnitude bins (dB) when present this frame. */
  spectrum: number[] | null;
}

export interface DeviceInfo {
  name: string;
  isDefault: boolean;
}

export interface StreamFormat {
  sampleRate: number;
  channels: number;
}

/** The error shape every Tauri command rejects with. */
export interface IpcError {
  code: string;
  message: string;
}
