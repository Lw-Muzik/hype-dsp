import { beforeEach, describe, expect, it } from "vitest";
import type { DeviceInfo, Frame, PlayerState } from "../protocol";
import { ytRef } from "../testKit";
import { CMD_TIMEOUT_MS, OVERLAY_MS, positionNow, RemoteSession, type RemoteView } from "./session";

const dev = (id: string, over: Partial<DeviceInfo> = {}): DeviceInfo => ({
  id,
  kind: "mobile",
  name: `Name ${id}`,
  platform: null,
  appVersion: null,
  caps: ["player", "yt"],
  online: true,
  lastSeenAt: "x",
  ...over,
});
const SELF = "me";
const ME = dev(SELF, { kind: "desktop" });
const PHONE = dev("phone");
const TAB = dev("tab");
const WEB = dev("web", { kind: "web", caps: [] });

const st = (seq: number, over: Partial<PlayerState> = {}): PlayerState => ({
  seq,
  track: ytRef("a"),
  playing: true,
  positionMs: 10_000,
  durationMs: 200_000,
  volume: 0.5,
  shuffle: false,
  repeat: "off",
  queue: [{ uid: "q1", track: ytRef("b") }],
  at: 1_000_000,
  ...over,
});

class Rig {
  hub = 1_000_000;
  sent: Frame[] = [];
  failSend = false;
  timers = new Map<number, { at: number; fn: () => void }>();
  n = 1;
  lost: DeviceInfo[] = [];
  views: (RemoteView | null)[] = [];
  s = new RemoteSession({
    hubNow: () => this.hub,
    send: (f) => {
      if (this.failSend) return Promise.reject(new Error("x"));
      this.sent.push(f);
      return Promise.resolve();
    },
    setTimeout: (fn, ms) => {
      const id = this.n++;
      this.timers.set(id, { at: this.hub + ms, fn });
      return id;
    },
    clearTimeout: (h) => void this.timers.delete(h as number),
    onChange: (v) => this.views.push(v),
    onTargetLost: (d) => this.lost.push(d),
  });
  tick(ms: number) {
    this.hub += ms;
    for (const [id, t] of [...this.timers]) {
      if (t.at <= this.hub) {
        this.timers.delete(id);
        t.fn();
      }
    }
  }
}
let r: Rig;
beforeEach(() => {
  r = new Rig();
  r.s.setSelfId(SELF);
  r.s.setConnected(true);
});

describe("remote mode", () => {
  it("is on when another online player is active", () => {
    r.s.onWelcome([ME, PHONE], "phone", { phone: st(1) });
    expect(r.s.target?.id).toBe("phone");
    expect(r.s.view()?.track).toEqual(ytRef("a"));
  });
  it.each([
    ["self active", [ME, PHONE], SELF],
    ["nobody active", [ME, PHONE], null],
    ["offline device", [ME, dev("phone", { online: false })], "phone"],
    ["a non-player", [ME, WEB], "web"],
    ["an unknown id", [ME], "ghost"],
  ])("is off when %s", (_n, devices, active) => {
    r.s.onWelcome(devices as DeviceInfo[], active as string | null, {});
    expect(r.s.target).toBeNull();
    expect(r.s.view()).toBeNull();
  });
  it("is off while disconnected", () => {
    r.s.onWelcome([ME, PHONE], "phone", {});
    r.s.setConnected(false);
    expect(r.s.target).toBeNull();
  });
  it("shows an empty idle view before the first state arrives", () => {
    r.s.onDevices([ME, PHONE], "phone");
    const v = r.s.view()!;
    expect(v.track).toBeNull();
    expect(v.playing).toBe(false);
  });
  it("lists online players only", () => {
    r.s.onDevices([ME, PHONE, WEB, dev("tab", { online: false })], null);
    expect(r.s.players.map((d) => d.id)).toEqual([SELF, "phone"]);
  });
});

describe("state tracking", () => {
  beforeEach(() => r.s.onWelcome([ME, PHONE, TAB], "phone", { phone: st(5), tab: st(3, { track: ytRef("t") }) }));
  it("drops older and equal seq per device", () => {
    r.s.onState("phone", st(4, { positionMs: 1 }));
    r.s.onState("phone", st(5, { positionMs: 2 }));
    expect(r.s.view()?.positionMs).toBe(10_000);
    r.s.onState("phone", st(6, { positionMs: 3 }));
    expect(r.s.view()?.positionMs).toBe(3);
  });
  it("tracks devices independently", () => {
    r.s.onState("tab", st(4, { positionMs: 99 }));
    expect(r.s.stateOf("tab")?.positionMs).toBe(99);
    expect(r.s.view()?.device.id).toBe("phone");
  });
  it("ignores its own state and unaddressed frames", () => {
    r.s.onState(SELF, st(9));
    r.s.onState(undefined, st(9));
    expect(r.s.stateOf(SELF)).toBeNull();
  });
  it("forgets a device that goes offline, and welcome resets everything", () => {
    r.s.onDevices([ME, PHONE, dev("tab", { online: false })], "phone");
    expect(r.s.stateOf("tab")).toBeNull();
    r.s.onWelcome([ME, PHONE], "phone", {});
    expect(r.s.stateOf("phone")).toBeNull();
  });
  it("after forgetting, an old seq is accepted again", () => {
    r.s.onDevices([ME, PHONE, dev("tab", { online: false })], "phone");
    r.s.onDevices([ME, PHONE, TAB], "phone");
    r.s.onState("tab", st(1));
    expect(r.s.stateOf("tab")?.seq).toBe(1);
  });
  it("stamps a missing `at` with the hub clock", () => {
    r.s.onState("phone", { ...st(7), at: undefined });
    expect(r.s.stateOf("phone")?.at).toBe(r.hub);
  });
});

describe("interpolation (§3.6)", () => {
  const v = { positionMs: 10_000, at: 1_000, playing: true, durationMs: 20_000 };
  it("adds elapsed hub time while playing", () => expect(positionNow(v, 3_500)).toBe(12_500));
  it("freezes while paused", () => expect(positionNow({ ...v, playing: false }, 9_000)).toBe(10_000));
  it("clamps to the duration", () => expect(positionNow(v, 99_000)).toBe(20_000));
  it("never goes negative (clock skew)", () => expect(positionNow({ ...v, positionMs: 0 }, 0)).toBe(0));
  it("has no upper bound without a duration", () =>
    expect(positionNow({ ...v, durationMs: null }, 99_000)).toBe(108_000));
});

describe("cmd", () => {
  beforeEach(() => r.s.onWelcome([ME, PHONE], "phone", { phone: st(5) }));
  it("sends a cmd with `to`, no `from`, and resolves on the matching ack", async () => {
    const p = r.s.cmd("phone", { type: "next" });
    const f = r.sent[0]!;
    expect(f).toMatchObject({ t: "cmd", to: "phone", cmd: { type: "next" } });
    expect(f).not.toHaveProperty("from");
    r.s.onAck((f as { id: string }).id, true);
    await expect(p).resolves.toEqual({ ok: true });
  });
  it("uses a distinct id per command and ignores other acks", async () => {
    const a = r.s.cmd("phone", { type: "next" });
    const b = r.s.cmd("phone", { type: "prev" });
    const [fa, fb] = r.sent as { id: string }[];
    expect(fa!.id).not.toBe(fb!.id);
    r.s.onAck("nope", true);
    r.s.onAck(fb!.id, false, "offline");
    await expect(b).resolves.toEqual({ ok: false, error: "offline" });
    r.s.onAck(fa!.id, true);
    await expect(a).resolves.toEqual({ ok: true });
  });
  it("times out after 11 s", async () => {
    const p = r.s.cmd("phone", { type: "next" });
    expect(CMD_TIMEOUT_MS).toBe(11_000);
    r.tick(10_999);
    r.tick(1);
    await expect(p).resolves.toEqual({ ok: false, error: "timeout" });
    // A late ack is harmless.
    r.s.onAck((r.sent[0] as { id: string }).id, true);
  });
  it("a send failure resolves failed", async () => {
    r.failSend = true;
    await expect(r.s.cmd("phone", { type: "next" })).resolves.toEqual({ ok: false, error: "failed" });
  });
  it("pending commands fail with offline when the connection drops", async () => {
    const p = r.s.cmd("phone", { type: "next" });
    r.s.setConnected(false);
    await expect(p).resolves.toEqual({ ok: false, error: "offline" });
  });
});

describe("optimistic overlay", () => {
  beforeEach(() => r.s.onWelcome([ME, PHONE], "phone", { phone: st(5) }));
  it("pause shows paused at once, frozen where it was", () => {
    r.tick(2_000);
    void r.s.cmd("phone", { type: "pause" });
    const v = r.s.view()!;
    expect(v.playing).toBe(false);
    expect(positionNow(v, r.hub + 5_000)).toBe(12_000);
  });
  it("resume shows playing", () => {
    r.s.onState("phone", st(6, { playing: false }));
    void r.s.cmd("phone", { type: "resume" });
    expect(r.s.view()?.playing).toBe(true);
  });
  it("seek shows the target position", () => {
    void r.s.cmd("phone", { type: "seek", positionMs: 77_000 });
    const v = r.s.view()!;
    expect(positionNow(v, r.hub)).toBe(77_000);
  });
  it("volume, shuffle, repeat show at once", () => {
    void r.s.cmd("phone", { type: "volume", value: 0.9 });
    void r.s.cmd("phone", { type: "shuffle", on: true });
    void r.s.cmd("phone", { type: "repeat", mode: "all" });
    const v = r.s.view()!;
    expect([v.volume, v.shuffle, v.repeat]).toEqual([0.9, true, "all"]);
  });
  it("a newer-seq state replaces it", () => {
    void r.s.cmd("phone", { type: "pause" });
    r.s.onState("phone", st(6, { playing: true }));
    expect(r.s.view()?.playing).toBe(true);
  });
  it("an older or equal seq state does not", () => {
    void r.s.cmd("phone", { type: "pause" });
    r.s.onState("phone", st(5, { playing: true }));
    expect(r.s.view()?.playing).toBe(false);
  });
  it("expires after 3 s", () => {
    void r.s.cmd("phone", { type: "pause" });
    r.tick(OVERLAY_MS);
    expect(r.s.view()?.playing).toBe(true);
    expect(r.views.at(-1)?.playing).toBe(true);
  });
  it("a failed command takes the guess back", async () => {
    const p = r.s.cmd("phone", { type: "pause" });
    r.s.onAck((r.sent[0] as { id: string }).id, false, "failed");
    await p;
    expect(r.s.view()?.playing).toBe(true);
  });
  it("commands without a visible guess leave the view alone", () => {
    void r.s.cmd("phone", { type: "next" });
    expect(r.s.view()?.playing).toBe(true);
  });
});

describe("target lost", () => {
  it("notifies when the remote device goes offline", () => {
    r.s.onWelcome([ME, PHONE], "phone", { phone: st(1) });
    r.s.onDevices([ME, dev("phone", { online: false })], "phone");
    expect(r.lost.map((d) => d.id)).toEqual(["phone"]);
    expect(r.s.view()).toBeNull();
  });
  it("notifies when the device vanishes from the list", () => {
    r.s.onWelcome([ME, PHONE], "phone", { phone: st(1) });
    r.s.onDevices([ME], "phone");
    expect(r.lost).toHaveLength(1);
  });
  it("does not notify when the active device just changes", () => {
    r.s.onWelcome([ME, PHONE, TAB], "phone", { phone: st(1) });
    r.s.onDevices([ME, PHONE, TAB], "tab");
    r.s.onDevices([ME, PHONE, TAB], SELF);
    expect(r.lost).toEqual([]);
  });
  it("does not notify for a plain disconnect", () => {
    r.s.onWelcome([ME, PHONE], "phone", {});
    r.s.setConnected(false);
    expect(r.lost).toEqual([]);
  });
});

describe("select", () => {
  it("sends a select frame", async () => {
    await r.s.select("phone");
    expect(r.sent[0]).toEqual({ t: "select", deviceId: "phone" });
  });
});
