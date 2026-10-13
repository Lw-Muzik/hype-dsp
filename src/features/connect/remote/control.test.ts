import { afterEach, describe, expect, it, vi } from "vitest";
import { ytRef } from "../testKit";
import { configureRemote, mediaToCommand, routeMediaCommand } from "./control";
import type { RemoteView } from "./session";

const view = (over: Partial<RemoteView> = {}): RemoteView => ({
  device: { id: "phone", kind: "mobile", name: "Pixel", platform: null, appVersion: null, caps: ["player"], online: true, lastSeenAt: "x" },
  seq: 1, track: ytRef("a"), playing: true, positionMs: 10_000, at: 1_000, durationMs: 200_000,
  volume: 1, shuffle: false, repeat: "off", queue: [], ...over,
});

afterEach(() => configureRemote(null));

describe("mediaToCommand", () => {
  it("maps explicit and toggle actions to explicit commands", () => {
    expect(mediaToCommand("toggle", null, view(), 1_000)).toEqual({ type: "pause" });
    expect(mediaToCommand("toggle", null, view({ playing: false }), 1_000)).toEqual({ type: "resume" });
    expect(mediaToCommand("play", null, view(), 1_000)).toEqual({ type: "resume" });
    expect(mediaToCommand("pause", null, view(), 1_000)).toEqual({ type: "pause" });
    expect(mediaToCommand("next", null, view(), 1_000)).toEqual({ type: "next" });
    expect(mediaToCommand("prev", null, view(), 1_000)).toEqual({ type: "prev" });
  });
  it("relative seeks start from the interpolated position", () => {
    expect(mediaToCommand("seekForward", null, view(), 3_000)).toEqual({ type: "seek", positionMs: 22_000 });
    expect(mediaToCommand("seekBackward", 5, view(), 1_000)).toEqual({ type: "seek", positionMs: 5_000 });
    expect(mediaToCommand("seek", 42, view(), 1_000)).toEqual({ type: "seek", positionMs: 42_000 });
  });
  it("unknown actions have no command", () => {
    expect(mediaToCommand("rewind", null, view(), 0)).toBeNull();
    expect(mediaToCommand("seek", null, view(), 0)).toBeNull();
  });
});

describe("routeMediaCommand", () => {
  it("stays local when not a remote", () => {
    const local = vi.fn();
    routeMediaCommand("toggle", null, local);
    expect(local).toHaveBeenCalledTimes(1);
  });
  it("drives the remote device instead of the local engine", () => {
    const cmd = vi.fn().mockResolvedValue({ ok: true });
    configureRemote({ view: () => view(), cmd, hubNow: () => 1_000, moveTo: vi.fn() });
    const local = vi.fn();
    routeMediaCommand("toggle", null, local);
    expect(local).not.toHaveBeenCalled();
    expect(cmd).toHaveBeenCalledWith("phone", { type: "pause" });
  });
  it("stays local when configured but in local mode", () => {
    configureRemote({ view: () => null, cmd: vi.fn(), hubNow: () => 0, moveTo: vi.fn() });
    const local = vi.fn();
    routeMediaCommand("next", null, local);
    expect(local).toHaveBeenCalled();
  });
});
