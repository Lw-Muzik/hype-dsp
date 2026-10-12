/** Pure display helpers for the Hype Connect card. */
import type { ConnectStatus } from "@/lib/ipc";
import type { DeviceInfo } from "./protocol";

export type Tone = "ok" | "busy" | "warn" | "off";

export interface ConnectionView {
  tone: Tone;
  title: string;
  detail: string;
  /** Offer "Reconnect" (the client stopped and won't retry on its own). */
  canReconnect: boolean;
}

export function describeConnection(status: ConnectStatus | null): ConnectionView {
  const view = (tone: Tone, title: string, detail: string, canReconnect = false) => ({
    tone,
    title,
    detail,
    canReconnect,
  });
  if (!status) return view("busy", "Starting…", "Checking Hype Connect.");
  if (!status.enabled) {
    return view("off", "Off", "Turn on to play music on this desktop from a browser.");
  }
  const c = status.connection;
  if (!c) return view("busy", "Starting…", "Opening the connection.");
  switch (c.state) {
    case "connected":
      return view(
        "ok",
        "Ready",
        "Pick this desktop in Hype Muzik on the web and the music plays here.",
      );
    case "connecting":
      return view("busy", "Connecting…", "Reaching the Hype Connect service.");
    case "waiting": {
      const secs = Math.max(1, Math.round(c.retryInMs / 1000));
      return view("warn", `Reconnecting in ${secs}s`, c.reason);
    }
    case "no_session":
      return view("busy", "Waiting for your account…", "Hype Connect starts once you're signed in.");
    case "stopped":
      switch (c.reason) {
        case "unlinked":
          return view(
            "warn",
            "Unlinked",
            "This desktop was removed from your devices. Reconnect to add it back.",
            true,
          );
        case "replaced":
          return view(
            "warn",
            "In use elsewhere",
            "Another copy of Hype Muzik on this computer took over Hype Connect.",
            true,
          );
        case "tls_unavailable":
          return view("warn", "Can't secure the connection", "Hype Connect couldn't set up TLS.", true);
        case "shutdown":
          return view("off", "Stopped", "Hype Connect isn't running.", true);
      }
  }
}

const PLATFORMS: Record<string, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
  android: "Android",
  ios: "iOS",
};

export function platformLabel(platform: string | null): string | null {
  if (!platform) return null;
  return PLATFORMS[platform.toLowerCase()] ?? platform;
}

/** "Chrome on macOS" is already the name a browser links under; players get
 *  their platform appended. */
export function deviceSubtitle(d: DeviceInfo, now: number): string {
  const parts = [platformLabel(d.platform)];
  parts.push(d.online ? "Online" : lastSeen(d.lastSeenAt, now));
  return parts.filter(Boolean).join(" · ");
}

export function lastSeen(iso: string, now: number): string {
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return "Offline";
  const mins = Math.floor((now - at) / 60_000);
  if (mins < 1) return "Seen just now";
  if (mins < 60) return `Seen ${mins} min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `Seen ${hours} h ago`;
  return `Seen ${Math.floor(hours / 24)} d ago`;
}

/** "483920" → "483 920", the way it's read aloud. */
export function formatCode(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/** Seconds until `iso`, never negative. */
export function secondsUntil(iso: string, now: number): number {
  const at = Date.parse(iso);
  return Number.isFinite(at) ? Math.max(0, Math.ceil((at - now) / 1000)) : 0;
}
