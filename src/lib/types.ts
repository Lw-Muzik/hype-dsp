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
  /** When true, scales boost down when low-band energy is already strong
   *  (anti-overload). `false` = static boost (today's default behavior). */
  adaptive: boolean;
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

/** Convolution (impulse-response) stage. Heavy IR data lives engine-side. */
export interface ConvolverState {
  enabled: boolean;
  wetDry: number;
  irGainDb: number;
  irId: string | null;
  irName: string | null;
  irSeconds: number;
  irTruncated: boolean;
}

/** Multiband compander (10-band LR compressor/expander); global params. */
export interface CompanderState {
  enabled: boolean;
  thresholdDb: number;
  ratio: number;
  kneeDb: number;
  attackMs: number;
  releaseMs: number;
  makeupDb: number;
  gateDb: number;
  expanderRatio: number;
}

/** Tube-style analog saturation (4× oversampled). */
export interface SaturationState {
  enabled: boolean;
  drive: number;
  mix: number;
}

export interface ScriptState {
  enabled: boolean;
  source: string;
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
  { id: "none", name: "None", roomSize: 0.0, decay: 0.0, damping: 0.0, preDelay: 0, diffusion: 0.0, wetDry: 0.0 },
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

/** Queue-playback behaviour: gapless transitions + crossfade between tracks. */
export interface PlaybackState {
  gapless: boolean;
  /** Crossfade duration in seconds (0 = off; implies gapless when > 0). */
  crossfadeSecs: number;
  /** Low-bandwidth mode: progressive streaming, no prefetch, bigger buffers. */
  dataSaver: boolean;
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
  convolver: ConvolverState;
  compander: CompanderState;
  saturation: SaturationState;
  script: ScriptState;
  headphone: HeadphoneCorrectionState;
  output: OutputState;
  playback: PlaybackState;
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

/** A named snapshot of the entire DSP enhancement chain. Mirrors `ChainPreset` in hm-core. */
export interface ChainPreset {
  id: string;
  name: string;
  state: EngineState;
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

/** A country in the Africa radio browser. */
export interface RadioCountry {
  /** ISO 3166-1 alpha-2 code (e.g. "NG"). */
  code: string;
  name: string;
}

/** A television channel from the world TV directory (iptv-org). The `url` is a
 * video stream (usually HLS) played by the native mpv window. */
export interface TvChannel {
  id: string;
  name: string;
  url: string;
  logo: string | null;
  /** Category from the playlist's `group-title` (e.g. "News"). */
  group: string | null;
  /** ISO 3166-1 alpha-2 country code, when known. */
  country: string | null;
  userAgent: string | null;
  referrer: string | null;
  /** Resolution hint parsed from the name (e.g. "720p"). */
  quality: string | null;
}

/** A browsable TV category (iptv-org bucket). */
export interface TvCategory {
  /** Slug used in the playlist URL (e.g. "news"). */
  id: string;
  name: string;
}

/** A country in the world TV browser. */
export interface TvCountry {
  /** ISO 3166-1 alpha-2 code. */
  code: string;
  name: string;
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

/** The server's license verdict (from the Management API). */
export interface LicenseInfo {
  state: "trial" | "expired" | "licensed" | "blocked";
  allowed: boolean;
  daysLeft: number;
  trialEndsAt: string | null;
  licensedUntil: string | null;
}

/** The signed-in account + its current entitlement. */
export interface AccountStatus {
  authenticated: boolean;
  email: string | null;
  name: string | null;
  license: LicenseInfo | null;
}

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
  /** Per-band compander gain-reduction in dB (10 values, ≤0), or null when idle. */
  companderGr: number[] | null;
}

export interface LibraryTrack {
  path: string;
  title: string;
  artist: string | null;
  album: string | null;
  /** Genre from the file's tags, used for the library's category filter. */
  genre: string | null;
  durationSecs: number | null;
}

/** One page of library tracks. `tracks` is the reachable subset; `scanned` is
 *  the DB rows read before availability filtering, so the loader advances its
 *  offset correctly even when an unplugged drive's rows are hidden. */
export interface LibraryPage {
  tracks: LibraryTrack[];
  scanned: number;
}

export interface Playlist {
  id: string;
  name: string;
}

export interface TransportProgress {
  positionSecs: number;
  durationSecs: number | null;
  paused: boolean;
  /** Whether the active source can be scrubbed (false for live radio). */
  seekable: boolean;
  buffering: boolean;
  downloadBps: number;
  rebufferCount: number;
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

/** How an output device is attached — drives the picker icon + speaker/headphone hint. */
export type OutputTransport =
  | "builtin"
  | "usb"
  | "bluetooth"
  | "hdmi"
  | "displayport"
  | "airplay"
  | "aggregate"
  | "virtual"
  | "thunderbolt"
  | "other";

/** A selectable system output device (from `audio_output_devices`). */
export interface OutputDevice {
  /** Core Audio AudioObjectID on macOS; 0 elsewhere. */
  id: number;
  /** Stable selection key (device UID on macOS, name elsewhere). */
  uid: string;
  name: string;
  transport: OutputTransport;
  isDefault: boolean;
  isAlive: boolean;
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

/** One connected cloud account. The same provider can have several (e.g. two
 *  Google accounts), so `id` — not `provider` — is the identity used to list,
 *  stream, and cache. `label` is the account's email / display name. */
export interface CloudAccount {
  id: string;
  provider: CloudProvider;
  label: string;
}

/** A folder or audio file inside a cloud account (browsed folder-by-folder). */
export interface CloudEntry {
  provider: CloudProvider;
  /** Which connected account this entry belongs to (its `CloudAccount.id`). */
  accountId: string;
  /** Folder/file handle: Drive object id, or Dropbox path. */
  id: string;
  name: string;
  isFolder: boolean;
  size: number;
  /** Parent folder name, set on the flat account-wide listing (else null). */
  folder?: string | null;
}

/** A flat account-wide audio listing. `fromCache` is true when it was served
 *  from the on-disk cache (so the caller can refresh it in the background). */
export interface CloudAudioPage {
  entries: CloudEntry[];
  fromCache: boolean;
}

/** A cloud track's embedded tags, read from the file's leading bytes. */
export interface CloudTrackMeta {
  title: string | null;
  artist: string | null;
  album: string | null;
  /** Front cover as a `data:` URI, if the file had embedded art. */
  cover: string | null;
}

export interface CloudStatus {
  /** Every connected account (any number per provider). */
  accounts: CloudAccount[];
  /** Whether OAuth credentials are configured for the provider. */
  googleConfigured: boolean;
  dropboxConfigured: boolean;
}

/* ---------------------------------------------------------- YouTube Music */

/** One of the signed-in account's playlists. */
export interface YtPlaylist {
  id: string;
  title: string;
  author: string;
  /** Null when YT Music reports it in a form the backend doesn't recognise. */
  trackCount: number | null;
  thumbnail: string | null;
}

/** One track inside a playlist. Unlike cloud files, YT Music hands us the tags
 *  up front, so there's no metadata pass — the listing is already complete. */
export interface YtTrack {
  videoId: string;
  title: string;
  artist: string | null;
  album: string | null;
  durationSecs: number | null;
  /** Cover art URL (https), used directly as an `<img src>`. */
  thumbnail: string | null;
  /** The playlist this track was listed under — the library's Folders facet
   *  groups by it, so playlists browse like folders. */
  playlistId: string;
  playlistTitle: string;
  /** Region-blocked / removed tracks stay listed (the playlist matches what
   *  the user sees on YouTube) but can't be played. */
  isAvailable: boolean;
}

/** Whether yt-dlp — which resolves every stream and download — is installed.
 *  Playlists browse without it; only playback and downloads need it. */
export interface YtDlpInfo {
  present: boolean;
  version: string | null;
  path: string | null;
  /** Without ffmpeg, downloads skip embedded tags/artwork. */
  haveFfmpeg: boolean;
}

export interface YtMusicStatus {
  signedIn: boolean;
  ytdlp: YtDlpInfo;
}

/** A whole-library listing. `fromCache` mirrors {@link CloudAudioPage}: true
 *  means it was served from the on-disk cache, so refresh it behind the UI. */
export interface YtMusicPage {
  playlists: YtPlaylist[];
  tracks: YtTrack[];
  fromCache: boolean;
}

/** Progress of a download (to this machine or on to a phone), per `videoId`. */
export interface YtDownloadProgress {
  videoId: string;
  /** "fetching" (YouTube → here), "sending" (here → phone), "done", "error". */
  phase: "fetching" | "sending" | "done" | "error";
  bytes: number;
  /** Absent until the transfer's length is known. */
  total: number | null;
  /** Set on `error` only (the backend omits it otherwise). */
  message?: string;
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
  /** The folder the file lives in on the phone, for folder browsing. */
  folder: string | null;
  hasArt: boolean;
}
