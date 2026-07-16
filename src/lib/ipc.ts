/**
 * Typed IPC layer.
 *
 * Every Tauri command is wrapped in a typed function here; components never
 * call `invoke` directly. Commands reject with an `IpcError`; `isIpcError`
 * narrows the unknown rejection so callers can surface `code`/`message`.
 * Streaming telemetry arrives over events, wrapped in `onEngineFrame` /
 * `onTransport`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AccountStatus,
  AppInfo,
  ChainPreset,
  CloudAccount,
  CloudAudioPage,
  CloudEntry,
  CloudProvider,
  CloudStatus,
  CloudTrackMeta,
  CompanderState,
  DeviceInfo,
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  IpcError,
  LibraryPage,
  LibraryTrack,
  LicenseStatus,
  MixerSnapshot,
  OutputDevice,
  OutputState,
  PhoneDevice,
  PhoneTrack,
  Playlist,
  RadioCountry,
  RadioStation,
  TvChannel,
  TvCategory,
  TvCountry,
  ExploreItem,
  ExploreSection,
  ExploreShelf,
  YtDownloadProgress,
  YtMusicPage,
  YtMusicStatus,
  YtTrack,
  ConvolverState,
  RoomState,
  SaturationState,
  SpatialMode,
  SurroundSpeakers,
  TrackMeta,
  TransportProgress,
} from "./types";

/* ----------------------------------------------------------------- commands */

/** App name, version, and engine schema revision. */
export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

/** System output devices (UID + transport + default flag) for the picker. */
export function outputDevices(): Promise<OutputDevice[]> {
  return invoke<OutputDevice[]>("audio_output_devices");
}

/** Make the given device (by UID) the system default output. The engine follows
 *  the default, so this moves the app's (and all system) audio to it. */
export function setDefaultOutput(uid: string): Promise<void> {
  return invoke<void>("audio_set_default_output", { uid });
}

/** System input (capture) devices. */
export function listInputDevices(): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("audio_list_input_devices");
}

/** Read the current engine state. */
export function engineGetState(): Promise<EngineState> {
  return invoke<EngineState>("engine_get_state");
}

/** Toggle global enhancement power. */
export function engineSetPower(power: boolean): Promise<void> {
  return invoke<void>("engine_set_power", { power });
}

/** Set master output volume (linear gain). */
export function engineSetMasterVolume(volume: number): Promise<void> {
  return invoke<void>("engine_set_master_volume", { volume });
}

/** Apply a manual 31-band EQ edit. */
export function engineSetEq(
  bands: number[],
  preGain: number,
  enabled: boolean,
): Promise<void> {
  return invoke<void>("engine_set_eq", { bands, preGain, enabled });
}

/** Configure the bass boost stage. */
export function engineSetBass(
  enabled: boolean,
  amount: number,
  harmonics: boolean,
  adaptive: boolean,
): Promise<void> {
  return invoke<void>("engine_set_bass", { enabled, amount, harmonics, adaptive });
}

/** Configure the spatializer (surround) stage. */
export function engineSetSpatializer(
  enabled: boolean,
  amount: number,
  mode: SpatialMode,
): Promise<void> {
  return invoke<void>("engine_set_spatializer", { enabled, amount, mode });
}

/** Configure the 3D-surround (virtual-speaker) stage. */
export function engineSetSurround3d(
  enabled: boolean,
  intensity: number,
  subwoofer: number,
  speakers: SurroundSpeakers,
): Promise<void> {
  return invoke<void>("engine_set_surround3d", {
    enabled,
    intensity,
    subwoofer,
    speakers,
  });
}

/** Configure the room-reverb ("room effects") stage. */
export function engineSetRoom(room: RoomState): Promise<void> {
  return invoke<void>("engine_set_room", { room });
}

/** Configure the convolution reverb / IR-correction stage. */
export function engineSetConvolver(convolver: ConvolverState): Promise<void> {
  return invoke<void>("engine_set_convolver", { convolver });
}

/** Configure the multiband compander stage. */
export function engineSetCompander(compander: CompanderState): Promise<void> {
  return invoke<void>("engine_set_compander", { compander });
}

/** Configure the tube-saturation stage. */
export function engineSetOutput(output: OutputState): Promise<void> {
  return invoke("engine_set_output", { output });
}

export function engineSetSaturation(saturation: SaturationState): Promise<void> {
  return invoke<void>("engine_set_saturation", { saturation });
}

export interface ConvolverIrInfo {
  name: string;
  seconds: number;
  truncated: boolean;
  channels: number;
}

/** Load an impulse-response file; returns metadata about what was loaded. */
export function engineConvolverLoadIr(path: string): Promise<ConvolverIrInfo> {
  return invoke<ConvolverIrInfo>("engine_convolver_load_ir", { path });
}

/* ------------------------------------------------------------ cloud music */

/** The connected cloud accounts + which providers are configured. */
export function cloudStatus(): Promise<CloudStatus> {
  return invoke<CloudStatus>("cloud_status");
}

/** Run the OAuth flow for a provider (opens the browser) and add the signed-in
 *  account. Connecting a second account of the same provider adds another. */
export function cloudConnect(provider: CloudProvider): Promise<CloudAccount> {
  return invoke<CloudAccount>("cloud_connect", { provider });
}

/** Forget one account's stored tokens (by account id). */
export function cloudDisconnect(accountId: string): Promise<void> {
  return invoke<void>("cloud_disconnect", { accountId });
}

/** List one cloud folder's contents (subfolders + audio); "" = account root. */
export function cloudList(
  accountId: string,
  folder: string,
): Promise<CloudEntry[]> {
  return invoke<CloudEntry[]>("cloud_list", { accountId, folder });
}

/** Every audio file in an account, flat (all folders), for the Player's unified
 *  library. Mirrors the mobile app's account-wide listing. The listing is
 *  cached on disk per account: by default a cached copy is returned instantly
 *  (with `fromCache: true`); pass `refresh` to re-list from the account and
 *  update the cache. */
export function cloudAllAudio(
  accountId: string,
  refresh = false,
): Promise<CloudAudioPage> {
  return invoke<CloudAudioPage>("cloud_all_audio", { accountId, refresh });
}

/** Every cached *text tag* for an account (no cover art), keyed by file id.
 *  Hydrates the library's titles/artists/albums instantly on launch without
 *  pulling every track's ~100 KB base64 cover into memory — covers resolve
 *  lazily per visible row via {@link cloudTrackCover}. */
export function cloudCachedTags(
  accountId: string,
): Promise<Record<string, CloudTrackMeta>> {
  return invoke<Record<string, CloudTrackMeta>>("cloud_cached_tags", {
    accountId,
  });
}

/** Read a cloud track's embedded tags (title/artist/album + cover) from the
 *  file's leading bytes. Cached on disk per file, so it's a one-time download. */
export function cloudTrackMetadata(
  accountId: string,
  fileId: string,
  name: string,
): Promise<CloudTrackMeta | null> {
  return invoke<CloudTrackMeta | null>("cloud_track_metadata", {
    accountId,
    fileId,
    name,
  });
}

/** Read a cloud track's text tags **only** (no cover). Backs the background
 *  library preload: still caches the full metadata on disk (so a later cover
 *  lookup is warm), but never ships the base64 cover over IPC or into the heap. */
export function cloudTrackTags(
  accountId: string,
  fileId: string,
  name: string,
): Promise<CloudTrackMeta | null> {
  return invoke<CloudTrackMeta | null>("cloud_track_tags", {
    accountId,
    fileId,
    name,
  });
}

/** Resolve just one cloud track's cover art (a `data:` URI), lazily per visible
 *  row — a warm cache hit once the tags preload has downloaded the file. */
export function cloudTrackCover(
  accountId: string,
  fileId: string,
  name: string,
): Promise<string | null> {
  return invoke<string | null>("cloud_track_cover", {
    accountId,
    fileId,
    name,
  });
}

/** Stream a cloud file through the enhancement chain. */
export function cloudPlay(accountId: string, fileId: string): Promise<void> {
  return invoke<void>("cloud_play", { accountId, fileId });
}

/** One track in a cloud crossfade/gapless queue (id + extension hint). */
export interface CloudQueueItem {
  id: string;
  ext?: string;
}

/** Play a cloud queue gaplessly/crossfading; URLs resolve lazily per track. All
 *  items must belong to the same account (`accountId`). */
export function playerPlayCloudQueue(
  accountId: string,
  items: CloudQueueItem[],
  start: number,
): Promise<void> {
  return invoke<void>("player_play_cloud_queue", { accountId, items, start });
}

/* ---------------------------------------------------------- YouTube Music */

/** Whether we're signed in, and whether yt-dlp/ffmpeg are installed. */
export function ytmusicStatus(): Promise<YtMusicStatus> {
  return invoke<YtMusicStatus>("ytmusic_status");
}

/** Open Google's sign-in window and resolve once a session appears. Rejects
 *  with code `cancelled` (window closed) or `timeout`. */
export function ytmusicSignIn(): Promise<YtMusicStatus> {
  return invoke<YtMusicStatus>("ytmusic_sign_in");
}

/** Forget the session and drop the cached listing. */
export function ytmusicSignOut(): Promise<void> {
  return invoke<void>("ytmusic_sign_out");
}

/** Every track across the account's playlists. Like {@link cloudAllAudio}, the
 *  listing is cached on disk: by default a cached copy returns instantly (with
 *  `fromCache: true`); pass `refresh` to re-list and update the cache. */
export function ytmusicAllTracks(refresh = false): Promise<YtMusicPage> {
  return invoke<YtMusicPage>("ytmusic_all_tracks", { refresh });
}

/** The mood/genre categories YouTube offers. */
export function ytmusicExploreCategories(): Promise<ExploreSection[]> {
  return invoke<ExploreSection[]>("ytmusic_explore_categories");
}

/** One category's shelves. Never cached — Explore is YouTube's live catalog, so
 *  each open is a fresh read (that's the point of browsing it). */
export function ytmusicExplorePage(params: string): Promise<ExploreShelf[]> {
  return invoke<ExploreShelf[]>("ytmusic_explore_page", { params });
}

/** The tracks behind one Explore item (playlist or album), ready to queue. */
export function ytmusicExploreTracks(item: ExploreItem): Promise<YtTrack[]> {
  return invoke<YtTrack[]>("ytmusic_explore_tracks", { item });
}

/** Stream one track through the enhancement chain. Passing the known length
 *  (seconds) makes the seek bar right from the first frame. */
export function ytmusicPlay(
  videoId: string,
  durationSecs?: number | null,
): Promise<void> {
  return invoke<void>("ytmusic_play", {
    videoId,
    durationSecs: durationSecs ?? null,
  });
}

/** One track in a YouTube Music crossfade/gapless queue. */
export interface YtQueueItem {
  videoId: string;
}

/** Play a YT Music queue gaplessly/crossfading; URLs resolve lazily per track
 *  (they're short-lived and IP-pinned, so they can't be resolved up front). */
export function playerPlayYtmusicQueue(
  items: YtQueueItem[],
  start: number,
): Promise<void> {
  return invoke<void>("player_play_ytmusic_queue", { items, start });
}

/** Download a track to this machine and index it into the library; returns the
 *  written path. Progress arrives over {@link onYtDownload}. */
export function ytmusicDownload(videoId: string): Promise<string> {
  return invoke<string>("ytmusic_download", { videoId });
}

/** Download a track and send it on to a paired phone. */
export function ytmusicDownloadToPhone(
  videoId: string,
  deviceId: string,
): Promise<void> {
  return invoke<void>("ytmusic_download_to_phone", { videoId, deviceId });
}

/** Send any already-local file to a paired phone. */
export function linkUpload(deviceId: string, path: string): Promise<void> {
  return invoke<void>("link_upload", { deviceId, path });
}

/** The folder downloads are written to. */
export function ytmusicDownloadDir(): Promise<string> {
  return invoke<string>("ytmusic_download_dir");
}

/** Set the download folder; `null` resets it to the default. Returns the
 *  folder now in effect. */
export function ytmusicSetDownloadDir(dir: string | null): Promise<string> {
  return invoke<string>("ytmusic_set_download_dir", { dir });
}

/** Subscribe to download progress. Every download reports on this one event,
 *  keyed by `videoId` (a `link_upload` of a local file keys by its path). */
export function onYtDownload(
  handler: (p: YtDownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<YtDownloadProgress>("ytmusic:download", (e) => handler(e.payload));
}

/* ------------------------------------------------------------- Phone Link */

/** Browse the LAN (~2.5 s) for phones sharing their library. */
export function linkDiscover(): Promise<PhoneDevice[]> {
  return invoke<PhoneDevice[]>("link_discover");
}

/** Phones we've already paired with (silent reconnect). */
export function linkPaired(): Promise<PhoneDevice[]> {
  return invoke<PhoneDevice[]>("link_paired");
}

/** Start continuous phone discovery — phones arrive over `onPhoneFound`. */
export function linkDiscoverStart(): Promise<void> {
  return invoke<void>("link_discover_start");
}

/** Stop continuous phone discovery. */
export function linkDiscoverStop(): Promise<void> {
  return invoke<void>("link_discover_stop");
}

/** Subscribe to phones as they're discovered on the LAN (live, no polling). */
export function onPhoneFound(
  handler: (device: PhoneDevice) => void,
): Promise<UnlistenFn> {
  return listen<PhoneDevice>("link:phone_found", (e) => handler(e.payload));
}

/** Fires when an already-paired phone (re)appears on the LAN — i.e. it's
 *  reachable now — so its library can be auto-synced into the unified library
 *  without a relaunch. Payload is the device id. */
export function onPairedOnline(
  handler: (deviceId: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("link:paired_online", (e) => handler(e.payload));
}

/** Pair with a phone using the 6-digit PIN it's showing. */
export function linkPair(
  host: string,
  port: number,
  name: string,
  deviceId: string,
  pin: string,
): Promise<PhoneDevice> {
  return invoke<PhoneDevice>("link_pair", { host, port, name, deviceId, pin });
}

/** Pair with a phone by its address (host:port) + PIN — no mDNS discovery
 *  needed (works when discovery can't see the phone, or across networks). */
export function linkPairAddress(
  host: string,
  port: number,
  pin: string,
): Promise<PhoneDevice> {
  return invoke<PhoneDevice>("link_pair_address", { host, port, pin });
}

/** Forget a paired phone. */
export function linkUnpair(deviceId: string): Promise<void> {
  return invoke<void>("link_unpair", { deviceId });
}

// ------------------------------------------- remote (cross-network) phone link

/** What the phone scans to pair across networks. */
export interface RemotePairingInfo {
  endpointId: string;
  pin: string;
  qr: string;
}

/** A paired remote phone's live status. */
export interface RemotePhoneStatus {
  id: string;
  name: string;
  online: boolean;
  port: number | null;
}

/** Open a remote pairing session; returns the QR payload + PIN to display. */
export function linkRemoteQr(): Promise<RemotePairingInfo> {
  return invoke<RemotePairingInfo>("link_remote_qr");
}

/** Close the open remote pairing session. */
export function linkRemoteCancel(): Promise<void> {
  return invoke<void>("link_remote_cancel");
}

/** Status of every paired remote phone. */
export function linkRemoteStatus(): Promise<RemotePhoneStatus[]> {
  return invoke<RemotePhoneStatus[]>("link_remote_status");
}

/** (Re)dial all known remote phones; returns the refreshed status list. */
export function linkRemoteConnect(): Promise<RemotePhoneStatus[]> {
  return invoke<RemotePhoneStatus[]>("link_remote_connect");
}

/** Forget a remote phone (drops its tunnel + both stores). */
export function linkRemoteForget(deviceId: string): Promise<void> {
  return invoke<void>("link_remote_forget", { deviceId });
}

/** Fires when a remote phone pairs / reconnects — re-scan the library. */
export function onRemoteConnected(
  handler: (deviceId: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("link:remote_connected", (e) => handler(e.payload));
}

/** Fetch a paired phone's track list. */
export function linkLibrary(deviceId: string): Promise<PhoneTrack[]> {
  return invoke<PhoneTrack[]>("link_library", { deviceId });
}

/** A track's artwork as a data URI, or null if it has none. */
export function linkArtwork(
  deviceId: string,
  trackId: string,
): Promise<string | null> {
  return invoke<string | null>("link_artwork", { deviceId, trackId });
}

/** A phone track's lyrics (a `.lrc` the user keeps next to the music, or
 *  embedded lyrics) as raw LRC/plain text, or null if the phone has none. */
export function linkLyrics(
  deviceId: string,
  trackId: string,
): Promise<string | null> {
  return invoke<string | null>("link_lyrics", { deviceId, trackId });
}

/** Stream one track from the phone through the enhancement chain. Passing the
 *  track's known length (seconds) makes the stream seekable and shows its
 *  duration straight away. */
export function linkPlay(
  deviceId: string,
  trackId: string,
  ext: string,
  durationSecs?: number | null,
): Promise<void> {
  return invoke<void>("link_play", {
    deviceId,
    trackId,
    ext,
    durationSecs: durationSecs ?? null,
  });
}

/** One track in a phone crossfade/gapless queue (id + extension). */
export interface PhoneQueueItem {
  id: string;
  ext: string;
}

/** Play a phone queue gaplessly/crossfading; URLs resolve lazily per track. */
export function linkPlayQueue(
  deviceId: string,
  items: PhoneQueueItem[],
  start: number,
): Promise<void> {
  return invoke<void>("link_play_queue", { deviceId, items, start });
}

/** Now-playing pushed by a phone casting to this desktop. */
export interface LinkNowPlaying {
  title: string;
  artist: string | null;
  source: string;
}

/** Subscribe to cast notifications from a paired phone. */
export function onLinkNowPlaying(
  handler: (np: LinkNowPlaying) => void,
): Promise<UnlistenFn> {
  return listen<LinkNowPlaying>("link:now_playing", (e) => handler(e.payload));
}

/** All bundled headphone correction profiles. */
export function profileList(): Promise<HeadphoneProfile[]> {
  return invoke<HeadphoneProfile[]>("profile_list");
}

/** Apply a headphone profile; returns it. */
export function profileSetActive(id: string): Promise<HeadphoneProfile> {
  return invoke<HeadphoneProfile>("profile_set_active", { id });
}

/** Clear the active headphone correction. */
export function profileClear(): Promise<void> {
  return invoke<void>("profile_clear");
}

/** List all presets (built-in first, then custom). */
export function eqListPresets(): Promise<EqPreset[]> {
  return invoke<EqPreset[]>("eq_list_presets");
}

/** Apply a preset to the engine; returns the applied preset. */
export function eqApplyPreset(id: string): Promise<EqPreset> {
  return invoke<EqPreset>("eq_apply_preset", { id });
}

/** Save the current curve as a new custom preset. */
export function eqSaveCustom(
  name: string,
  bands: number[],
  preGain: number,
): Promise<EqPreset> {
  return invoke<EqPreset>("eq_save_custom", { name, bands, preGain });
}

/** Update an existing custom preset. */
export function eqUpdate(preset: EqPreset): Promise<void> {
  return invoke<void>("eq_update", { preset });
}

/** Delete a custom preset. */
export function eqDelete(id: string): Promise<void> {
  return invoke<void>("eq_delete", { id });
}

/** Decode and play a local file through the chain. */
export function playerPlayFile(path: string): Promise<void> {
  return invoke<void>("player_play_file", { path });
}

/** Stream and play an internet radio URL. */
export function playerPlayRadio(url: string): Promise<void> {
  return invoke<void>("player_play_radio", { url });
}

/** Play a list of local files as a gapless/crossfading queue from `start`. */
export function playerPlayQueue(paths: string[], start: number): Promise<void> {
  return invoke<void>("player_play_queue", { paths, start });
}

/** Update gapless + crossfade playback behaviour. */
export function engineSetPlayback(
  gapless: boolean,
  crossfadeSecs: number,
): Promise<void> {
  return invoke<void>("engine_set_playback", { gapless, crossfadeSecs });
}

/** Toggle Data Saver / low-bandwidth streaming mode. */
export function engineSetDataSaver(on: boolean): Promise<void> {
  return invoke<void>("engine_set_data_saver", { on });
}

/** Subscribe to the gapless queue's current track index. */
export function onQueueIndex(
  handler: (index: number) => void,
): Promise<UnlistenFn> {
  return listen<number>("engine:queue_index", (e) => handler(e.payload));
}

/** Stop playback. */
export function playerStop(): Promise<void> {
  return invoke<void>("player_stop");
}

/** Pause playback (keeps position). */
export function playerPause(): Promise<void> {
  return invoke<void>("player_pause");
}

/** Resume playback. */
export function playerResume(): Promise<void> {
  return invoke<void>("player_resume");
}

/** Seek to `secs` within the current track. */
export function playerSeek(secs: number): Promise<void> {
  return invoke<void>("player_seek", { secs });
}

/** Capture the default input device through the chain (dev stand-in). */
export function playerPlayCapture(): Promise<void> {
  return invoke<void>("player_play_capture");
}

/** Whether true system-wide capture (a signed virtual device) is installed. */
export function captureVirtualAvailable(): Promise<boolean> {
  return invoke<boolean>("capture_virtual_available");
}

/** Equalize system-wide audio through the chain: macOS taps + re-renders every
 *  app; Linux/Windows re-route all output through a virtual device. */
export function playerPlaySystemAudio(): Promise<void> {
  return invoke<void>("player_play_system_audio");
}

/** Stop system-wide equalization and restore normal audio routing. */
export function stopSystemAudio(): Promise<void> {
  return invoke<void>("stop_system_audio");
}

/** Whether system-wide equalization is available (macOS tap / Linux PipeWire
 *  or PulseAudio / Windows bundled virtual device). */
export function systemAudioAvailable(): Promise<boolean> {
  return invoke<boolean>("system_audio_available");
}

/** Per-OS readiness of system-wide EQ for the Settings card. */
export interface SystemAudioStatus {
  /** The OS supports system-wide EQ at all (show the card). */
  supported: boolean;
  /** Ready to enable now (Windows: the bundled audio driver is installed). */
  available: boolean;
  /** Windows-only: the bundled virtual-audio driver is installed. */
  driverInstalled: boolean;
  /** This OS routes through a bundled driver the user may need to install. */
  needsDriver: boolean;
}

/** Per-OS system-EQ readiness (supported / available / driver state) in one call. */
export function systemAudioStatus(): Promise<SystemAudioStatus> {
  return invoke<SystemAudioStatus>("system_audio_status");
}

/** Runtime state of system-wide EQ. `"recovering"` means a transient failure
 *  (e.g. a macOS tap stall under heavy load, or a device change) is being
 *  recovered from in the background — audio is restored but currently unequalised
 *  — rather than the EQ having silently stopped. */
export type SystemEqRuntimeStatus = "active" | "recovering" | "disabled";

/** Poll the current runtime state of system-wide EQ (active / recovering /
 *  disabled), so the UI can surface background recovery instead of a silent stall. */
export function systemEqStatus(): Promise<SystemEqRuntimeStatus> {
  return invoke<SystemEqRuntimeStatus>("system_eq_status");
}

/** Install the bundled Windows virtual-audio driver (prompts for admin via UAC).
 *  No-op on platforms that need no driver. Re-query {@link systemAudioStatus}
 *  afterwards to confirm the device enumerated. */
export function systemAudioInstallDriver(): Promise<void> {
  return invoke<void>("system_audio_install_driver");
}

/** Whether the native MilkDrop visualizer sidecar is bundled in this build. */
export function visualizerAvailable(): Promise<boolean> {
  return invoke<boolean>("visualizer_available");
}

/** Every bundled `.milk` preset name (file stem), sorted. */
export function visualizerPresetNames(): Promise<string[]> {
  return invoke<string[]>("visualizer_preset_names");
}

/** Open the visualizer window, streaming audio to it and (optionally) starting
 *  on a given preset. */
export function visualizerStart(opts?: {
  fps?: number;
  beat?: number;
  presetSecs?: number;
  preset?: string;
}): Promise<void> {
  return invoke<void>("visualizer_start", {
    fps: opts?.fps ?? null,
    beat: opts?.beat ?? null,
    presetSecs: opts?.presetSecs ?? null,
    preset: opts?.preset ?? null,
  });
}

/** Switch the open visualizer window to a preset (a `.milk` file stem). */
export function visualizerSetPreset(preset: string): Promise<void> {
  return invoke<void>("visualizer_set_preset", { preset });
}

/** Close the visualizer window. */
export function visualizerStop(): Promise<void> {
  return invoke<void>("visualizer_stop");
}

/** Whether the visualizer window is currently open. */
export function visualizerIsOpen(): Promise<boolean> {
  return invoke<boolean>("visualizer_is_open");
}

/* ----------------------------------------------- in-app visualizer scenes */

/** A selectable in-app (Canvas/WebGL) visualizer. */
export interface SceneInfo {
  id: string;
  name: string;
  /** "2d" (Canvas) or "3d" (WebGL/Three.js). */
  kind: "2d" | "3d";
}

/** The registry of in-app visualizer scenes (backend source of truth). */
export function sceneList(): Promise<SceneInfo[]> {
  return invoke<SceneInfo[]>("scene_list");
}

/** The currently selected scene id (persisted by the backend). */
export function sceneSelected(): Promise<string> {
  return invoke<string>("scene_selected");
}

/** Select a scene; the backend persists it. */
export function sceneSelect(id: string): Promise<void> {
  return invoke<void>("scene_select", { id });
}

/** Whether audio is currently playing. */
export function playerIsPlaying(): Promise<boolean> {
  return invoke<boolean>("player_is_playing");
}

/* ----------------------------------------------------------------- mixer */

export function mixerListSessions(): Promise<MixerSnapshot> {
  return invoke<MixerSnapshot>("mixer_list_sessions");
}
export function mixerSetVolume(id: string, gain: number): Promise<void> {
  return invoke<void>("mixer_set_volume", { id, gain });
}
export function mixerSetMuted(id: string, muted: boolean): Promise<void> {
  return invoke<void>("mixer_set_muted", { id, muted });
}

/* --------------------------------------------------------------- license */

export function licenseStatus(): Promise<LicenseStatus> {
  return invoke<LicenseStatus>("license_status");
}
export function licenseActivate(key: string): Promise<LicenseStatus> {
  return invoke<LicenseStatus>("license_activate", { key });
}
export function licenseDeactivate(): Promise<void> {
  return invoke<void>("license_deactivate");
}

/* --------------------------------------------------------------- account */

export function accountStatus(): Promise<AccountStatus> {
  return invoke<AccountStatus>("account_status");
}
/** Create a passwordless account — the server emails a sign-in code. */
export function accountSignup(email: string, name?: string): Promise<void> {
  return invoke<void>("account_signup", { email, name: name ?? null });
}
/** Email a sign-in code to an existing account. */
export function accountRequestOtp(email: string): Promise<void> {
  return invoke<void>("account_request_otp", { email });
}
/** Verify an emailed code → starts the session, returns the account status. */
export function accountVerify(
  email: string,
  code: string,
): Promise<AccountStatus> {
  return invoke<AccountStatus>("account_verify", { email, code });
}
export function accountLogout(): Promise<void> {
  return invoke<void>("account_logout");
}
export function accountHeartbeat(
  platform: string,
  appVersion: string,
): Promise<void> {
  return invoke<void>("account_heartbeat", { platform, appVersion });
}

/* ----------------------------------------------------------------- stems */

export interface StemStatus {
  /** The separator (htdemucs model + ONNX Runtime) is installed. */
  available: boolean;
  /** This track is already separated (cached) — arming is instant. */
  separated: boolean;
  /** CoreML (Neural Engine / GPU) is driving inference, vs. CPU fallback. */
  accelerated: boolean;
}

export function stemsStatus(trackPath: string): Promise<StemStatus> {
  return invoke<StemStatus>("stems_status", { trackPath });
}
/** Arm stems for the current track: separate it (cached if available) and swap
 *  the stems in at the live playhead. Emits `stems:progress` while it runs. */
export function stemsArm(trackPath: string): Promise<void> {
  return invoke<void>("stems_arm", { trackPath });
}
/** Set a stem's gain live (0 = muted). Stem order: 0 vocals, 1 drums, 2 bass, 3 other. */
export function stemsSetGain(stem: number, gain: number): Promise<void> {
  return invoke<void>("stems_set_gain", { stem, gain });
}
/** Reset every stem to unity — the mix sounds like the original track again. */
export function stemsReset(): Promise<void> {
  return invoke<void>("stems_reset");
}
export function stemsGains(): Promise<number[]> {
  return invoke<number[]>("stems_gains");
}
/** Separation progress, 0..1. */
export function onStemsProgress(
  handler: (value: number) => void,
): Promise<UnlistenFn> {
  return listen<number>("stems:progress", (e) => handler(e.payload));
}

/* --------------------------------------------------------------- library */

/** Recursively scan a folder into the library; returns tracks added. */
export function libraryScan(dir: string): Promise<number> {
  return invoke<number>("library_scan", { dir });
}

/** Re-read tags for every track already in the library; returns the count.
 *  Backfills tags for libraries scanned before tag extraction existed. */
export function libraryRefreshTags(): Promise<number> {
  return invoke<number>("library_refresh_tags");
}

/** A track recognized by audio fingerprint (AcoustID). */
export interface RecognitionResult {
  title: string | null;
  artist: string | null;
  album: string | null;
  score: number;
  written: boolean;
}

/** Identify one local track by audio fingerprint and fill in missing tags. */
export function identifyTrack(path: string): Promise<RecognitionResult | null> {
  return invoke<RecognitionResult | null>("identify_track", { path });
}

/** Fingerprint + identify every library track missing info; tags them in place.
 *  Returns the number successfully tagged. Emits `library:scan_progress`. */
export function libraryIdentifyMissing(): Promise<number> {
  return invoke<number>("library_identify_missing");
}

/** List all library tracks at once (back-compat; the Library UI pages instead). */
export function libraryList(): Promise<LibraryTrack[]> {
  return invoke<LibraryTrack[]>("library_list");
}

/** Total track count, for showing a load-progress fraction. */
export function libraryCount(): Promise<number> {
  return invoke<number>("library_count");
}

/** Count of tracks whose file is currently reachable — probed on focus to
 *  detect a drive being plugged in or ejected without a needless reload. */
export function libraryAvailableCount(): Promise<number> {
  return invoke<number>("library_available_count");
}

/** One ordered page of the library (`title` order), reachable tracks only, for
 *  incremental loading. `scanned` < `limit` means the end has been reached. */
export function libraryListPage(offset: number, limit: number): Promise<LibraryPage> {
  return invoke<LibraryPage>("library_list_page", { offset, limit });
}

/* ----------------------------------------------- open from the file manager */

/** Import audio files opened from the OS file manager (or "Open With") into the
 *  library and return the resolved tracks, so the caller can enqueue and play
 *  them. Non-audio / unreadable paths are dropped. */
export function openFiles(paths: string[]): Promise<LibraryTrack[]> {
  return invoke<LibraryTrack[]>("open_files", { paths });
}

/** Drain audio paths the OS handed the app before the UI was ready (a cold
 *  launch or "Open With" before the window mounted). `[]` when none pending. */
export function takePendingOpenFiles(): Promise<string[]> {
  return invoke<string[]>("take_pending_open");
}

/** Subscribe to audio files opened while the app is already running (warm). */
export function onOpenFiles(
  handler: (paths: string[]) => void,
): Promise<UnlistenFn> {
  return listen<string[]>("app:open_files", (e) => handler(e.payload));
}

/** Remove a track from the library. */
export function libraryRemove(path: string): Promise<void> {
  return invoke<void>("library_remove", { path });
}

/** A track's embedded cover art as a data URI, or null if it has none.
 *  Read on demand so the scan stays fast; the UI caches results per path. */
export function libraryArtwork(path: string): Promise<string | null> {
  return invoke<string | null>("library_artwork", { path });
}

/** Resolve lyrics for a track (.lrc sidecar / embedded for local files, else
 *  an online lookup). Returns raw LRC or plain text, or null if none found. */
export function lyricsFetch(
  title: string,
  artist: string | null,
  durationSecs: number | null,
  path: string | null,
): Promise<string | null> {
  return invoke<string | null>("lyrics_fetch", {
    title,
    artist: artist ?? null,
    durationSecs: durationSecs ?? null,
    path: path ?? null,
  });
}

/** Progress of an in-flight library scan. */
export interface LibraryScanProgress {
  done: number;
  total: number;
}

/** Subscribe to library scan progress (emitted while a folder is importing). */
export function onLibraryScanProgress(
  handler: (p: LibraryScanProgress) => void,
): Promise<UnlistenFn> {
  return listen<LibraryScanProgress>("library:scan_progress", (e) =>
    handler(e.payload),
  );
}

export function playlistList(): Promise<Playlist[]> {
  return invoke<Playlist[]>("playlist_list");
}
export function playlistCreate(name: string): Promise<Playlist> {
  return invoke<Playlist>("playlist_create", { name });
}
export function playlistRename(id: string, name: string): Promise<void> {
  return invoke<void>("playlist_rename", { id, name });
}
export function playlistDelete(id: string): Promise<void> {
  return invoke<void>("playlist_delete", { id });
}
export function playlistTracks(id: string): Promise<LibraryTrack[]> {
  return invoke<LibraryTrack[]>("playlist_tracks", { id });
}
export function playlistAdd(id: string, path: string): Promise<void> {
  return invoke<void>("playlist_add", { id, path });
}
export function playlistRemove(id: string, path: string): Promise<void> {
  return invoke<void>("playlist_remove", { id, path });
}
export function playlistReorder(id: string, paths: string[]): Promise<void> {
  return invoke<void>("playlist_reorder", { id, paths });
}

/* ----------------------------------------------------------------- radio */

/** Search the radio directory (falls back to the bundled seed offline). */
export function radioSearch(query: string): Promise<RadioStation[]> {
  return invoke<RadioStation[]>("radio_search", { query });
}

/** The African countries available in the radio browser. */
export function radioAfricanCountries(): Promise<RadioCountry[]> {
  return invoke<RadioCountry[]>("radio_african_countries");
}

/** Every station for a country (ISO alpha-2 code), most-popular first. */
export function radioByCountry(code: string): Promise<RadioStation[]> {
  return invoke<RadioStation[]>("radio_by_country", { code });
}
export function radioFavoritesList(): Promise<RadioStation[]> {
  return invoke<RadioStation[]>("radio_favorites_list");
}
export function radioFavoriteAdd(station: RadioStation): Promise<void> {
  return invoke<void>("radio_favorite_add", { station });
}
export function radioFavoriteRemove(id: string): Promise<void> {
  return invoke<void>("radio_favorite_remove", { id });
}

/* -------------------------------------------------------------------- tv */

/** Search the world TV directory (falls back to the bundled seed offline). */
export function tvSearch(query: string): Promise<TvChannel[]> {
  return invoke<TvChannel[]>("tv_search", { query });
}

/** Every channel for a country (ISO 3166-1 alpha-2 code). */
export function tvByCountry(code: string): Promise<TvChannel[]> {
  return invoke<TvChannel[]>("tv_by_country", { code });
}

/** Every channel for a category (iptv-org slug, e.g. "news"). */
export function tvByCategory(id: string): Promise<TvChannel[]> {
  return invoke<TvChannel[]>("tv_by_category", { id });
}

/** The browsable TV categories. */
export function tvCategories(): Promise<TvCategory[]> {
  return invoke<TvCategory[]>("tv_categories");
}

/** The world country list for the browse grid. */
export function tvCountries(): Promise<TvCountry[]> {
  return invoke<TvCountry[]>("tv_countries");
}

export function tvFavoritesList(): Promise<TvChannel[]> {
  return invoke<TvChannel[]>("tv_favorites_list");
}

export function tvFavoriteAdd(channel: TvChannel): Promise<void> {
  return invoke<void>("tv_favorite_add", { channel });
}

export function tvFavoriteRemove(id: string): Promise<void> {
  return invoke<void>("tv_favorite_remove", { id });
}

/** The in-app playback URL for a channel — a loopback HLS-proxy URL the embedded
 * `<video>`/hls.js loads (the proxy adds the stream's headers + CORS). */
export function tvStreamUrl(channel: TvChannel): Promise<string> {
  return invoke<string>("tv_stream_url", { channel });
}

/** Open a native folder picker; returns the chosen directory. */
export async function pickFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

/** Subscribe to transport progress (~10 fps while playing). */
export function onProgress(
  handler: (p: TransportProgress) => void,
): Promise<UnlistenFn> {
  return listen<TransportProgress>("engine:progress", (e) => handler(e.payload));
}

/** A transport action from the OS media controls (Control Center / SMTC / MPRIS). */
export interface MediaCommand {
  /** play | pause | toggle | next | prev | stop | seek | seekForward | seekBackward */
  action: string;
  /** Absolute seek target, or seek delta (seconds), when applicable. */
  secs: number | null;
}

/** Subscribe to OS media-control actions (hardware keys, Control Center, etc.). */
export function onMediaCommand(
  handler: (cmd: MediaCommand) => void,
): Promise<UnlistenFn> {
  return listen<MediaCommand>("media:command", (e) => handler(e.payload));
}

/* ------------------------------------------------------------------- events */

/** Subscribe to real-time engine meter/spectrum frames. */
export function onEngineFrame(
  handler: (frame: EngineFrame) => void,
): Promise<UnlistenFn> {
  return listen<EngineFrame>("engine:frame", (event) => handler(event.payload));
}

/** Subscribe to play/stop transitions. */
export function onTransport(
  handler: (playing: boolean) => void,
): Promise<UnlistenFn> {
  return listen<boolean>("engine:transport", (event) => handler(event.payload));
}

/** Subscribe to decoded now-playing metadata (tags + cover art) per track. */
export function onNowPlaying(
  handler: (meta: TrackMeta) => void,
): Promise<UnlistenFn> {
  return listen<TrackMeta>("engine:now_playing", (event) => handler(event.payload));
}

/* ------------------------------------------------------------------ dialogs */

/** Import a GraphicEQ curve string into the engine and return the resolved bands + preGain. */
export interface EqImportResult {
  bands: number[];
  preGain: number;
}

export function engineEqImportGraphic(curve: string): Promise<EqImportResult> {
  return invoke<EqImportResult>("engine_eq_import_graphic", { curve });
}

/** Import a ViPER/JamesDSP DDC (.vdc) file by path → resolved bands + preGain. */
export function engineEqImportVdc(path: string): Promise<EqImportResult> {
  return invoke<EqImportResult>("engine_eq_import_vdc", { path });
}

/** Names of all bundled ViPER DDC presets (sorted), for the EQ library browser. */
export function ddcList(): Promise<string[]> {
  return invoke<string[]>("ddc_list");
}

/** Apply a bundled ViPER DDC preset by name → resolved bands + preGain. */
export function engineEqApplyDdc(name: string): Promise<EqImportResult> {
  return invoke<EqImportResult>("engine_eq_apply_ddc", { name });
}

/** Open a native file picker for an audio file; returns the chosen path. */
export async function pickAudioFile(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Audio", extensions: ["wav"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/* --------------------------------------------------------- chain presets */

/** List all saved whole-chain presets. */
export function chainPresetList(): Promise<ChainPreset[]> {
  return invoke<ChainPreset[]>("chain_preset_list");
}

/** Save the current engine state as a named whole-chain preset. */
export function chainPresetSave(name: string): Promise<ChainPreset> {
  return invoke<ChainPreset>("chain_preset_save", { name });
}

/** Apply a saved whole-chain preset by id (preserves current power + volume). */
export function chainPresetApply(id: string): Promise<void> {
  return invoke<void>("chain_preset_apply", { id });
}

/** Delete a saved whole-chain preset by id. */
export function chainPresetDelete(id: string): Promise<void> {
  return invoke<void>("chain_preset_delete", { id });
}

/** Export a saved whole-chain preset to a JSON file at `path`. */
export function chainPresetExport(id: string, path: string): Promise<void> {
  return invoke<void>("chain_preset_export", { id, path });
}

/** Import a whole-chain preset from a JSON file at `path`; returns the stored preset. */
export function chainPresetImport(path: string): Promise<ChainPreset> {
  return invoke<ChainPreset>("chain_preset_import", { path });
}

/* ------------------------------------------------------------------- errors */

/** Narrow an unknown command rejection to the structured IPC error shape. */
export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as { code: unknown }).code === "string" &&
    typeof (value as { message: unknown }).message === "string"
  );
}

/** Best-effort human-readable message from any command rejection. */
export function ipcErrorMessage(value: unknown): string {
  if (isIpcError(value)) return value.message;
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  return "Unexpected error";
}
