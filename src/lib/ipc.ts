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
  DeviceInfo,
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  IpcError,
  SpatialMode,
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

/** Stop playback. */
export function playerStop(): Promise<void> {
  return invoke<void>("player_stop");
}

/** Whether audio is currently playing. */
export function playerIsPlaying(): Promise<boolean> {
  return invoke<boolean>("player_is_playing");
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
