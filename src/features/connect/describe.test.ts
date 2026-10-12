import { describe, expect, it } from "vitest";
import type { ConnectStatus } from "@/lib/ipc";
import {
  describeConnection,
  deviceSubtitle,
  formatCode,
  lastSeen,
  platformLabel,
  secondsUntil,
} from "./describe";

const status = (connection: unknown, enabled = true): ConnectStatus =>
  ({ enabled, deviceId: "d", connection }) as unknown as ConnectStatus;

describe("describeConnection", () => {
  it("null status is starting", () => {
    expect(describeConnection(null)).toMatchObject({ tone: "busy", canReconnect: false });
  });

  it("disabled is off regardless of connection", () => {
    expect(describeConnection(status({ state: "connected" }, false))).toMatchObject({ tone: "off", title: "Off" });
  });

  it("connected is ready", () => {
    expect(describeConnection(status({ state: "connected" }))).toMatchObject({ tone: "ok", title: "Ready" });
  });

  it("waiting counts down in whole seconds, at least 1", () => {
    expect(
      describeConnection(status({ state: "waiting", retryInMs: 4400, reason: "boom" })),
    ).toMatchObject({ tone: "warn", title: "Reconnecting in 4s", detail: "boom" });
    expect(describeConnection(status({ state: "waiting", retryInMs: 10, reason: "" })).title).toBe(
      "Reconnecting in 1s",
    );
  });

  it("stopped states offer Reconnect; `unlinked` says why", () => {
    for (const reason of ["unlinked", "replaced", "tls_unavailable", "shutdown"]) {
      expect(describeConnection(status({ state: "stopped", reason })).canReconnect).toBe(true);
    }
    expect(describeConnection(status({ state: "stopped", reason: "unlinked" })).title).toBe("Unlinked");
    expect(describeConnection(status({ state: "connecting" })).canReconnect).toBe(false);
    expect(describeConnection(status({ state: "no_session" })).tone).toBe("busy");
  });
});

describe("helpers", () => {
  it("platformLabel maps known platforms and passes unknown ones through", () => {
    expect(platformLabel("macos")).toBe("macOS");
    expect(platformLabel("Android")).toBe("Android");
    expect(platformLabel("haiku")).toBe("haiku");
    expect(platformLabel(null)).toBeNull();
  });

  it("formatCode groups six digits as 3+3 and leaves anything else", () => {
    expect(formatCode("483920")).toBe("483 920");
    expect(formatCode("1234")).toBe("1234");
  });

  it("lastSeen buckets", () => {
    const now = Date.parse("2026-10-12T12:00:00Z");
    const ago = (ms: number) => new Date(now - ms).toISOString();
    expect(lastSeen(ago(10_000), now)).toBe("Seen just now");
    expect(lastSeen(ago(5 * 60_000), now)).toBe("Seen 5 min ago");
    expect(lastSeen(ago(3 * 3_600_000), now)).toBe("Seen 3 h ago");
    expect(lastSeen(ago(2 * 86_400_000), now)).toBe("Seen 2 d ago");
    expect(lastSeen("garbage", now)).toBe("Offline");
  });

  it("deviceSubtitle shows Online, else last seen, after the platform", () => {
    const now = Date.parse("2026-10-12T12:00:00Z");
    const base = {
      id: "1",
      kind: "mobile" as const,
      name: "P",
      platform: "android",
      appVersion: null,
      caps: [],
      lastSeenAt: new Date(now - 120_000).toISOString(),
    };
    expect(deviceSubtitle({ ...base, online: true }, now)).toBe("Android · Online");
    expect(deviceSubtitle({ ...base, online: false }, now)).toBe("Android · Seen 2 min ago");
    expect(deviceSubtitle({ ...base, platform: null, online: true }, now)).toBe("Online");
  });

  it("secondsUntil rounds up and never goes negative", () => {
    const now = 1_000_000;
    expect(secondsUntil(new Date(now + 1500).toISOString(), now)).toBe(2);
    expect(secondsUntil(new Date(now - 5000).toISOString(), now)).toBe(0);
    expect(secondsUntil("x", now)).toBe(0);
  });
});
