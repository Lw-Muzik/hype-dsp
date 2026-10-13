import { beforeEach, describe, expect, it, vi } from "vitest";
import { ytmusicItem, localItem, type QueueItem } from "@/stores/engine";
import { queueItemToRef } from "../refs";
import type { Command, DeviceInfo } from "../protocol";
import { DEVICE, libTrack, ytTrack } from "../testKit";
import { MAX_ITEMS_PER_PLAY, Router, selectPlayable, configureRouter, routePlay, routeEnqueue } from "./router";
import type { CmdResult } from "./session";

const PHONE: DeviceInfo = {
  id: "phone", kind: "mobile", name: "Pixel", platform: null, appVersion: null,
  caps: ["player", "yt"], online: true, lastSeenAt: "x",
};
const yt = (id: string): QueueItem => ytmusicItem(ytTrack(id));
const loc = (p: string): QueueItem => localItem(libTrack(p));
const station: QueueItem = {
  id: "http://r", source: "radio", title: "Radio One", artist: null, album: null, durationSecs: null, radioUrl: "http://r",
} as QueueItem;
const cloud = { ...yt("c"), source: "cloud", id: "cloud-1", title: "Cloudy" } as unknown as QueueItem;

let target: DeviceInfo | null;
let cmds: { to: string; cmd: Command }[];
let result: CmdResult;
let answer: boolean;
const deps = {
  target: () => target,
  toRef: (i: QueueItem) => queueItemToRef(i, DEVICE),
  cmd: vi.fn(async (to: string, cmd: Command) => {
    cmds.push({ to, cmd });
    return result;
  }),
  confirm: vi.fn(async () => answer),
  info: vi.fn(),
  error: vi.fn(),
};
const router = new Router(deps);

beforeEach(() => {
  target = PHONE;
  cmds = [];
  result = { ok: true };
  answer = false;
  vi.clearAllMocks();
});

describe("local mode", () => {
  it("runs the local behaviour and sends nothing", async () => {
    target = null;
    const local = vi.fn();
    await router.play({ items: [yt("a")], index: 0, local });
    await router.enqueue({ items: [yt("a")], mode: "end", local });
    expect(local).toHaveBeenCalledTimes(2);
    expect(cmds).toEqual([]);
    expect(deps.confirm).not.toHaveBeenCalled();
  });
});

describe("play in remote mode", () => {
  it("sends the refs and the index, and does not run local", async () => {
    const local = vi.fn();
    await router.play({ items: [yt("a"), yt("b"), yt("c")], index: 1, local });
    expect(local).not.toHaveBeenCalled();
    expect(cmds).toHaveLength(1);
    expect(cmds[0]!.to).toBe("phone");
    expect(cmds[0]!.cmd).toMatchObject({ type: "play", index: 1 });
    expect((cmds[0]!.cmd as { items: unknown[] }).items).toHaveLength(3);
    expect(cmds[0]!.cmd).not.toHaveProperty("radio");
  });
  it("sends radio:true for a single tapped song", async () => {
    await router.play({ items: [yt("a")], index: 0, radio: true, local: vi.fn() });
    expect(cmds[0]!.cmd).toMatchObject({ type: "play", radio: true });
  });
  it("does not send radio when more than one item survives", async () => {
    await router.play({ items: [yt("a"), yt("b")], index: 0, radio: true, local: vi.fn() });
    expect(cmds[0]!.cmd).not.toHaveProperty("radio");
  });
  it("drops what the phone can't play, re-bases the index and says how many", async () => {
    await router.play({ items: [loc("/a"), yt("b"), station, yt("c")], index: 3, local: vi.fn() });
    const c = cmds[0]!.cmd as { items: { videoId: string }[]; index: number };
    expect(c.items.map((i) => i.videoId)).toEqual(["b", "c"]);
    expect(c.index).toBe(1);
    expect(deps.info).toHaveBeenCalledWith(expect.stringContaining("2 tracks stayed"));
  });
  it("asks to play here when nothing is left; yes plays locally", async () => {
    answer = true;
    const local = vi.fn();
    await router.play({ items: [loc("/a")], index: 0, local });
    expect(deps.confirm).toHaveBeenCalledWith(expect.stringContaining("only on this computer"));
    expect(deps.confirm).toHaveBeenCalledWith(expect.stringContaining("Local /a"));
    expect(local).toHaveBeenCalledTimes(1);
    expect(cmds).toEqual([]);
  });
  it("asks about cloud and radio too; no does nothing", async () => {
    const local = vi.fn();
    await router.play({ items: [station], index: 0, local });
    await router.play({ items: [cloud], index: 0, local });
    expect(deps.confirm).toHaveBeenCalledTimes(2);
    expect(local).not.toHaveBeenCalled();
    expect(cmds).toEqual([]);
  });
  it("toasts the ack error", async () => {
    result = { ok: false, error: "offline" };
    await router.play({ items: [yt("a")], index: 0, local: vi.fn() });
    expect(deps.error).toHaveBeenCalledWith("Pixel is offline.");
  });
  it("does not say anything on success when everything was sent", async () => {
    await router.play({ items: [yt("a")], index: 0, local: vi.fn() });
    expect(deps.info).not.toHaveBeenCalled();
    expect(deps.error).not.toHaveBeenCalled();
  });
  it("caps a huge list around the tapped item", async () => {
    const items = Array.from({ length: 1000 }, (_, i) => yt(`v${i}`));
    await router.play({ items, index: 600, local: vi.fn() });
    const c = cmds[0]!.cmd as { items: { videoId: string }[]; index: number };
    expect(c.items).toHaveLength(MAX_ITEMS_PER_PLAY);
    expect(c.items[c.index]!.videoId).toBe("v600");
  });
});

describe("enqueue in remote mode", () => {
  it("sends enqueue with the mode", async () => {
    await router.enqueue({ items: [yt("a")], mode: "next", local: vi.fn() });
    expect(cmds[0]).toMatchObject({ to: "phone", cmd: { type: "enqueue", mode: "next" } });
  });
  it("offers the local queue when nothing can go", async () => {
    answer = true;
    const local = vi.fn();
    await router.enqueue({ items: [loc("/a")], mode: "end", local });
    expect(local).toHaveBeenCalledTimes(1);
    expect(cmds).toEqual([]);
  });
  it("reports dropped items", async () => {
    await router.enqueue({ items: [yt("a"), loc("/x")], mode: "end", local: vi.fn() });
    expect(deps.info).toHaveBeenCalledWith(expect.stringContaining("1 track stayed"));
  });
});

describe("selectPlayable", () => {
  const toRef = deps.toRef;
  it("falls forward, then back, when the requested item is dropped", () => {
    expect(selectPlayable([yt("a"), loc("/b"), yt("c")], 1, toRef).start).toBe(1);
    expect(selectPlayable([yt("a"), yt("b"), loc("/c")], 2, toRef).start).toBe(1);
  });
  it("empty when nothing is playable", () => {
    expect(selectPlayable([loc("/a")], 0, toRef)).toMatchObject({ refs: [], dropped: 1 });
  });
});

describe("the app's router", () => {
  it("runs local when unconfigured and routes when configured", () => {
    const local = vi.fn();
    configureRouter(null);
    routePlay({ items: [yt("a")], index: 0, local });
    routeEnqueue({ items: [yt("a")], mode: "end", local });
    expect(local).toHaveBeenCalledTimes(2);
    configureRouter(router);
    routePlay({ items: [yt("a")], index: 0, local });
    expect(local).toHaveBeenCalledTimes(2);
    expect(deps.cmd).toHaveBeenCalled();
    configureRouter(null);
  });
});
