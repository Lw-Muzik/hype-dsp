/**
 * Typed IPC layer.
 *
 * Every Tauri command is wrapped in a typed function here; components never
 * call `invoke` directly. Commands reject with an `IpcError`; `isIpcError`
 * narrows the unknown rejection so callers can surface `code`/`message`.
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppInfo, DeviceInfo, IpcError } from "./types";

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
