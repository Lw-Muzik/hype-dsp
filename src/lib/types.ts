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

/** On/off state for each virtual loudspeaker in the 3D Surround ring. */
export interface SurroundSpeakers {
  frontL: boolean;
  frontR: boolean;
  sideL: boolean;
  sideR: boolean;
  surroundL: boolean;
  surroundR: boolean;
}

export interface Surround3DState {
  enabled: boolean;
  /** 0.0 = dry, 1.0 = full virtual-surround. */
  intensity: number;
  /** LFE / subwoofer level, 0.0–1.0. */
  subwoofer: number;
  speakers: SurroundSpeakers;
}

/** Room reverb ("room effects"). Scalars are 0–1 except preDelay (ms). */
export interface RoomState {
  enabled: boolean;
  roomSize: number;
  decay: number;
  damping: number;
  preDelay: number;
  diffusion: number;
  wetDry: number;
  activePresetId: string | null;
}

/** A built-in room reverb preset (mirrors the Hype mobile presets). */
export interface RoomPreset {
  id: string;
  name: string;
  roomSize: number;
  decay: number;
  damping: number;
  preDelay: number;
  diffusion: number;
  wetDry: number;
}

export const ROOM_PRESETS: readonly RoomPreset[] = [
  { id: "small", name: "Small Room", roomSize: 0.2, decay: 0.25, damping: 0.6, preDelay: 3, diffusion: 0.5, wetDry: 0.25 },
  { id: "medium", name: "Medium Room", roomSize: 0.4, decay: 0.4, damping: 0.45, preDelay: 8, diffusion: 0.55, wetDry: 0.3 },
  { id: "large", name: "Large Room", roomSize: 0.6, decay: 0.55, damping: 0.35, preDelay: 15, diffusion: 0.6, wetDry: 0.35 },
  { id: "hall", name: "Hall", roomSize: 0.75, decay: 0.65, damping: 0.3, preDelay: 25, diffusion: 0.65, wetDry: 0.4 },
  { id: "cathedral", name: "Cathedral", roomSize: 0.9, decay: 0.8, damping: 0.5, preDelay: 40, diffusion: 0.7, wetDry: 0.4 },
  { id: "plate", name: "Plate", roomSize: 0.35, decay: 0.5, damping: 0.1, preDelay: 2, diffusion: 0.8, wetDry: 0.35 },
  { id: "studio", name: "Studio", roomSize: 0.15, decay: 0.2, damping: 0.65, preDelay: 2, diffusion: 0.4, wetDry: 0.2 },
  { id: "chamber", name: "Chamber", roomSize: 0.45, decay: 0.45, damping: 0.35, preDelay: 12, diffusion: 0.55, wetDry: 0.3 },
  { id: "arena", name: "Arena", roomSize: 1.0, decay: 0.9, damping: 0.55, preDelay: 60, diffusion: 0.65, wetDry: 0.35 },
  { id: "concert", name: "Concert", roomSize: 0.8, decay: 0.7, damping: 0.35, preDelay: 35, diffusion: 0.65, wetDry: 0.4 },
];

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
  surround3d: Surround3DState;
  room: RoomState;
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

export interface MixerSnapshot {
  supported: boolean;
  unavailableReason: string | null;
  sessions: AppSession[];
}

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

export interface LibraryTrack {
  path: string;
  title: string;
  artist: string | null;
  album: string | null;
  durationSecs: number | null;
}

export interface Playlist {
  id: string;
  name: string;
}

export interface TransportProgress {
  positionSecs: number;
  durationSecs: number | null;
  paused: boolean;
}

/** Now-playing metadata extracted from the decoded track (tags + cover art). */
export interface TrackMeta {
  title: string | null;
  artist: string | null;
  album: string | null;
  /** Embedded cover art as a data URI, if present. */
  cover: string | null;
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

export type CloudProvider = "googleDrive" | "dropbox";

/** A folder or audio file inside a cloud account (browsed folder-by-folder). */
export interface CloudEntry {
  provider: CloudProvider;
  /** Folder/file handle: Drive object id, or Dropbox path. */
  id: string;
  name: string;
  isFolder: boolean;
  size: number;
}

export interface CloudStatus {
  googleConnected: boolean;
  dropboxConnected: boolean;
  /** Whether OAuth credentials are configured for the provider. */
  googleConfigured: boolean;
  dropboxConfigured: boolean;
}

/* ------------------------------------------------------------- Phone Link */

/** A phone discovered on the LAN, or one we've already paired with. */
export interface PhoneDevice {
  /** Stable per-install id advertised by the phone. */
  id: string;
  name: string;
  host: string;
  port: number;
}

/** One track in a paired phone's library. */
export interface PhoneTrack {
  /** On-device song id (used to request the stream). */
  id: string;
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  /** File extension, e.g. "mp3" — appended to the stream URL. */
  ext: string;
  hasArt: boolean;
}
