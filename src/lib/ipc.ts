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
  IpcError,
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
