import { beforeEach, describe, expect, it } from "vitest";
import type { DeviceInfo } from "../protocol";
import { CLAIM_TIMEOUT_MS, Takeover } from "./takeover";

const dev = (id: string, over: Partial<DeviceInfo> = {}): DeviceInfo => ({
  id,
  kind: "mobile",
  name: id,
  platform: null,
  appVersion: null,
  caps: ["player", "yt"],
  online: true,
  lastSeenAt: "2026-01-01T00:00:00Z",
  ...over,
});
const SELF = "me";
const ME = dev(SELF, { kind: "desktop" });
const PHONE = dev("phone");
const WEB = dev("web", { kind: "web", caps: [] });

class Rig {
  t = 1_000;
  timers = new Map<number, { at: number; fn: () => void }>();
  nextTimer = 1;
  selects: string[] = [];
  pauses = 0;
  tk = new Takeover({
    now: () => this.t,
    setTimeout: (fn, ms) => {
      const id = this.nextTimer++;
      this.timers.set(id, { at: this.t + ms, fn });
      return id;
    },
    clearTimeout: (h) => void this.timers.delete(h as number),
    sendSelect: (id) => this.selects.push(id),
    pauseLocal: () => void (this.pauses += 1),
  });
  advance(ms: number) {
    const end = this.t + ms;
    for (;;) {
      const due = [...this.timers.entries()].filter(([, v]) => v.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      this.timers.delete(due[0]);
      this.t = due[1].at;
      due[1].fn();
    }
    this.t = end;
  }
}

let r: Rig;
beforeEach(() => {
  r = new Rig();
  r.tk.setSelfId(SELF);
  r.tk.setConnected(true);
});

describe("claim (a): not playing -> playing", () => {
  it("selects itself when another online player is active", () => {
    r.tk.onDevices([ME, PHONE], "phone");
    r.tk.setPlaying(true);
    expect(r.selects).toEqual([SELF]);
    expect(r.tk.isClaiming).toBe(true);
  });
  it("selects itself when nobody is active", () => {
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true);
    expect(r.selects).toEqual([SELF]);
  });
  it("does nothing when it is already the active device", () => {
    r.tk.onDevices([ME, PHONE], SELF);
    r.tk.setPlaying(true);
    expect(r.selects).toEqual([]);
    expect(r.tk.isClaiming).toBe(false);
  });
  it("only an edge claims; staying playing does not", () => {
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true);
    r.advance(10_000);
    r.tk.setPlaying(true);
    expect(r.selects).toHaveLength(1);
  });
  it("does nothing while disconnected", () => {
    r.tk.setConnected(false);
    r.tk.setPlaying(true);
    expect(r.selects).toEqual([]);
  });
  it("claims when the connection comes up while already playing and nobody is active (b)", () => {
    r.tk.setConnected(false);
    r.tk.setPlaying(true);
    r.tk.setConnected(true);
    r.tk.onDevices([ME], null);
    expect(r.selects).toEqual([SELF]);
  });
});

describe("claim (b): welcome/devices while playing", () => {
  beforeEach(() => {
    // Playing before any claim could go out (no id yet / just connected).
    r.tk.setConnected(false);
    r.tk.setPlaying(true);
    r.tk.setConnected(true);
  });
  it.each([
    ["null", [ME, PHONE], null],
    ["offline", [ME, dev("phone", { online: false })], "phone"],
    ["not a player", [ME, WEB], "web"],
    ["unknown id", [ME], "ghost"],
  ])("claims when the active device is %s", (_n, devices, active) => {
    r.tk.onDevices(devices as DeviceInfo[], active as string | null);
    expect(r.selects).toEqual([SELF]);
  });
  it("does not claim when another online player is active; it yields instead", () => {
    r.tk.onDevices([ME, PHONE], "phone");
    expect(r.selects).toEqual([]);
    expect(r.pauses).toBe(1);
  });
  it("does not claim when idle", () => {
    const idle = new Rig();
    idle.tk.setSelfId(SELF);
    idle.tk.setConnected(true);
    idle.tk.onDevices([ME], null);
    expect(idle.selects).toEqual([]);
  });
});

describe("claiming ends", () => {
  beforeEach(() => {
    r.tk.onDevices([ME, PHONE], "phone");
    r.tk.setPlaying(true);
  });
  it("when devices names this device active", () => {
    r.tk.onDevices([ME, PHONE], SELF);
    expect(r.tk.isClaiming).toBe(false);
    expect(r.pauses).toBe(0);
  });
  it("on error not_a_player (and then still applies the yield check)", () => {
    r.tk.onError("not_a_player");
    expect(r.tk.isClaiming).toBe(false);
    expect(r.pauses).toBe(1); // phone is still the active online player
  });
  it("ignores other error codes", () => {
    r.tk.onError("rate_limited");
    expect(r.tk.isClaiming).toBe(true);
  });
  it("after 3 s, then re-evaluates against the latest list", () => {
    expect(CLAIM_TIMEOUT_MS).toBe(3_000);
    r.advance(2_999);
    expect(r.tk.isClaiming).toBe(true);
    expect(r.pauses).toBe(0);
    r.advance(1);
    expect(r.tk.isClaiming).toBe(false);
    expect(r.pauses).toBe(1);
  });
  it("a user who paused meanwhile is not paused again (nothing to yield)", () => {
    r.tk.setPlaying(false);
    r.tk.onError("not_a_player");
    expect(r.pauses).toBe(0);
  });
  it("on disconnect, without pausing", () => {
    r.tk.setConnected(false);
    expect(r.tk.isClaiming).toBe(false);
    r.advance(5_000);
    expect(r.pauses).toBe(0);
  });
  it("a timeout does not claim again, even if nobody is active", () => {
    r.tk.onDevices([ME, PHONE], null); // postponed, claiming
    r.advance(CLAIM_TIMEOUT_MS + 5_000);
    expect(r.selects).toEqual([SELF]);
    expect(r.pauses).toBe(0);
  });
});

describe("yield", () => {
  it("pauses when playing and another online player becomes active", () => {
    r.tk.onDevices([ME, PHONE], SELF);
    r.tk.setPlaying(true);
    r.tk.onDevices([ME, PHONE], "phone");
    expect(r.pauses).toBe(1);
  });
  it("does not pause when not playing", () => {
    r.tk.onDevices([ME, PHONE], "phone");
    expect(r.pauses).toBe(0);
  });
  it("does not yield to an offline player or a non-player", () => {
    r.tk.onDevices([ME, WEB], SELF);
    r.tk.setPlaying(true);
    r.tk.onDevices([ME, WEB], "web");
    r.tk.onDevices([ME, dev("phone", { online: false })], "phone");
    expect(r.pauses).toBe(0);
  });
  it("postpones while claiming and uses the latest list when claiming ends", () => {
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true); // claim
    r.tk.onDevices([ME, PHONE], "phone"); // someone else won first: postponed
    expect(r.pauses).toBe(0);
    r.tk.onDevices([ME, PHONE], null); // latest list: nobody
    r.advance(CLAIM_TIMEOUT_MS);
    expect(r.pauses).toBe(0);
  });
});

describe("two players racing (last select wins)", () => {
  it("the loser yields once its claim ends", () => {
    // Both devices start at once; the hub applies mine first and the phone's last.
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true);
    expect(r.selects).toEqual([SELF]);
    r.tk.onDevices([ME, PHONE], SELF); // my select applied
    expect(r.tk.isClaiming).toBe(false);
    r.tk.onDevices([ME, PHONE], "phone"); // the phone's applied after
    expect(r.pauses).toBe(1);
  });
  it("the winner sees the loser's intermediate list while claiming and ignores it", () => {
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true);
    r.tk.onDevices([ME, PHONE], "phone"); // phone's select landed first
    expect(r.pauses).toBe(0);
    r.tk.onDevices([ME, PHONE], SELF); // mine landed last: I win
    expect(r.pauses).toBe(0);
    expect(r.tk.isClaiming).toBe(false);
  });
});

describe("rate limit: at most one claim per 3 s", () => {
  it("a second edge inside 3 s sends nothing", () => {
    r.tk.onDevices([ME, PHONE], "phone");
    r.tk.setPlaying(true);
    r.tk.onError("not_a_player");
    r.tk.setPlaying(false);
    r.advance(2_999);
    r.tk.setPlaying(true);
    expect(r.selects).toHaveLength(1);
  });
  it("a second edge at exactly 3 s claims again", () => {
    r.tk.onDevices([ME, PHONE], "phone");
    r.tk.setPlaying(true);
    r.tk.onError("not_a_player");
    r.tk.setPlaying(false);
    r.advance(3_000);
    r.tk.setPlaying(true);
    expect(r.selects).toHaveLength(2);
  });
  it("a devices-triggered claim (b) is limited too", () => {
    r.tk.onDevices([ME, PHONE], null);
    r.tk.setPlaying(true);
    r.tk.onError("not_a_player");
    r.advance(1_000);
    r.tk.onDevices([ME, PHONE], null);
    expect(r.selects).toHaveLength(1);
    r.advance(2_000);
    r.tk.onDevices([ME, PHONE], null);
    expect(r.selects).toHaveLength(2);
  });
});
