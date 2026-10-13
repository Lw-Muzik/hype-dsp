/**
 * The UI's handle on remote mode: send a transport command to the device the
 * app is controlling, read the hub clock, move playback, and route the OS
 * media keys. Wired once by `useHypeConnect`; before that (or with Connect
 * off) every call is a no-op and the media keys stay local.
 */
import type { Command } from "../protocol";
import { ackMessage } from "./router";
import { positionNow, type CmdResult, type RemoteView } from "./session";
import { useRemoteStore } from "./store";
import { toast } from "@/stores/toast";

export interface RemoteControl {
  /** The view of the device being controlled (with the optimistic overlay). */
  view(): RemoteView | null;
  cmd(to: string, command: Command): Promise<CmdResult>;
  hubNow(): number;
  moveTo(deviceId: string): Promise<unknown>;
}

let control: RemoteControl | null = null;

export function configureRemote(c: RemoteControl | null): void {
  control = c;
}

/** The hub's clock now (the local clock before Connect is up). */
export function remoteHubNow(): number {
  return control?.hubNow() ?? Date.now();
}

/** Send `command` to the device being controlled; toasts a failure. */
export async function sendRemote(command: Command): Promise<CmdResult | null> {
  const view = control?.view();
  if (!control || !view) return null;
  const result = await control.cmd(view.device.id, command);
  if (!result.ok) toast.error(ackMessage(result.error, view.device.name));
  return result;
}

export async function playOnDevice(deviceId: string): Promise<void> {
  await control?.moveTo(deviceId);
}

/** The media-key / tray actions the engine understands. */
export type MediaAction =
  | "play"
  | "pause"
  | "toggle"
  | "next"
  | "prev"
  | "stop"
  | "seek"
  | "seekForward"
  | "seekBackward";

/** The remote command for a media key, or null if it has none. */
export function mediaToCommand(
  action: string,
  secs: number | null,
  view: RemoteView,
  hubNowMs: number,
): Command | null {
  const here = positionNow(view, hubNowMs);
  switch (action) {
    case "play":
      return { type: "resume" };
    case "pause":
      return { type: "pause" };
    case "toggle":
      return { type: view.playing ? "pause" : "resume" };
    case "next":
      return { type: "next" };
    case "prev":
      return { type: "prev" };
    case "stop":
      return { type: "pause" };
    case "seek":
      return secs === null ? null : { type: "seek", positionMs: Math.max(0, secs * 1000) };
    case "seekForward":
      return { type: "seek", positionMs: here + (secs ?? 10) * 1000 };
    case "seekBackward":
      return { type: "seek", positionMs: Math.max(0, here - (secs ?? 10) * 1000) };
    default:
      return null;
  }
}

/**
 * OS media keys and tray buttons: while this app is a remote they drive the
 * other device (a media key must not resume the stale local queue and so take
 * the music back by accident). Otherwise `local` runs as before.
 */
export function routeMediaCommand(action: string, secs: number | null, local: () => void): void {
  const view = control?.view() ?? useRemoteStore.getState().view;
  if (!control || !view) {
    local();
    return;
  }
  const command = mediaToCommand(action, secs, view, control.hubNow());
  if (command) void sendRemote(command);
}
