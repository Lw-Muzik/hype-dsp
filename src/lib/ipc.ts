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
  AppInfo,
  CloudEntry,
  CloudProvider,
  CloudStatus,
  CloudTrackMeta,
  DeviceInfo,
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  IpcError,
  LibraryTrack,
  LicenseStatus,
  MixerSnapshot,
  PhoneDevice,
  PhoneTrack,
  Playlist,
  RadioStation,
  RoomState,
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

/** System output (playback) devices. */
export function listOutputDevices(): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("audio_list_output_devices");
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
): Promise<void> {
  return invoke<void>("engine_set_bass", { enabled, amount, harmonics });
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

/* ------------------------------------------------------------ cloud music */

/** Which cloud providers are configured and connected. */
export function cloudStatus(): Promise<CloudStatus> {
  return invoke<CloudStatus>("cloud_status");
}

/** Run the OAuth flow for a provider (opens the browser). */
export function cloudConnect(provider: CloudProvider): Promise<void> {
  return invoke<void>("cloud_connect", { provider });
}

/** Forget a provider's stored tokens. */
export function cloudDisconnect(provider: CloudProvider): Promise<void> {
  return invoke<void>("cloud_disconnect", { provider });
}

/** List one cloud folder's contents (subfolders + audio); "" = account root. */
export function cloudList(
  provider: CloudProvider,
  folder: string,
): Promise<CloudEntry[]> {
  return invoke<CloudEntry[]>("cloud_list", { provider, folder });
}

/** Every audio file in the account, flat (all folders), for the Player's
 *  unified library. Mirrors the mobile app's account-wide listing. */
export function cloudAllAudio(provider: CloudProvider): Promise<CloudEntry[]> {
  return invoke<CloudEntry[]>("cloud_all_audio", { provider });
}

/** Read a cloud track's embedded tags (title/artist/album + cover) from the
 *  file's leading bytes. Cached on disk per file, so it's a one-time download. */
export function cloudTrackMetadata(
  provider: CloudProvider,
  fileId: string,
  name: string,
): Promise<CloudTrackMeta | null> {
  return invoke<CloudTrackMeta | null>("cloud_track_metadata", {
    provider,
    fileId,
    name,
  });
}

/** Stream a cloud file through the enhancement chain. */
export function cloudPlay(provider: CloudProvider, fileId: string): Promise<void> {
  return invoke<void>("cloud_play", { provider, fileId });
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

/** Forget a paired phone. */
export function linkUnpair(deviceId: string): Promise<void> {
  return invoke<void>("link_unpair", { deviceId });
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

/** Equalize system-wide audio via a Core Audio process tap (macOS 14.4+). */
export function playerPlaySystemAudio(): Promise<void> {
  return invoke<void>("player_play_system_audio");
}

/** Whether system-wide capture via process taps is available (macOS 14.4+). */
export function systemAudioAvailable(): Promise<boolean> {
  return invoke<boolean>("system_audio_available");
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

/** List all library tracks. */
export function libraryList(): Promise<LibraryTrack[]> {
  return invoke<LibraryTrack[]>("library_list");
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
export function radioFavoritesList(): Promise<RadioStation[]> {
  return invoke<RadioStation[]>("radio_favorites_list");
}
export function radioFavoriteAdd(station: RadioStation): Promise<void> {
  return invoke<void>("radio_favorite_add", { station });
}
export function radioFavoriteRemove(id: string): Promise<void> {
  return invoke<void>("radio_favorite_remove", { id });
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

/** Open a native file picker for an audio file; returns the chosen path. */
export async function pickAudioFile(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Audio", extensions: ["wav"] }],
  });
  return typeof selected === "string" ? selected : null;
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
