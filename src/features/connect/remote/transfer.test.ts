import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, PlayerState, TrackRef } from "../protocol";
import type { StateBody } from "../statePublisher";
import { localRef, ytRef } from "../testKit";
import { buildTransfer, moveTo, playableOn, type TransferDeps } from "./transfer";
import type { CmdResult, StampedState } from "./session";

const SELF = "me";
const PHONE = "phone";
const TAB = "tab";
const NAMES: Record<string, string> = { me: "This computer", phone: "Pixel", tab: "Tablet" };

let calls: string[];
let source: string;
let remote: Record<string, StampedState>;
let local: StateBody;
let cmdResult: CmdResult;
let hereOk: boolean;
let hereCmd: Command | null;

const q = (uid: string, track: TrackRef = ytRef(uid)) => ({ uid, track });
const body = (over: Partial<StateBody> = {}): StateBody => ({
  track: ytRef("cur"), playing: true, positionMs: 30_000, durationMs: 200_000, volume: 1,
  shuffle: false, repeat: "off", queue: [q("n1"), q("n2")], ...over,
});
const stamped = (over: Partial<PlayerState> = {}): StampedState => ({ seq: 1, at: 1_000, ...body(), ...over }) as StampedState;

const deps: TransferDeps = {
  selfId: () => SELF,
  sourceId: () => source,
  stateOf: (id) => remote[id] ?? null,
  localState: () => local,
  hubNow: () => 5_000,
  deviceName: (id) => NAMES[id] ?? id,
  cmd: vi.fn(async (to, cmd) => {
    calls.push(`cmd:${to}:${cmd.type}`);
    if (cmd.type === "play") hereCmd = cmd;
    return cmdResult;
  }),
  select: vi.fn(async (id) => void calls.push(`select:${id}`)),
  playHere: vi.fn(async (cmd) => {
    calls.push("playHere");
    hereCmd = cmd;
    return hereOk ? ({ ok: true } as const) : ({ ok: false, error: "x" } as const);
  }),
  pauseLocal: vi.fn(() => void calls.push("pauseLocal")),
  info: vi.fn(),
  error: vi.fn(),
};

beforeEach(() => {
  calls = [];
  source = SELF;
  remote = {};
  local = body();
  cmdResult = { ok: true };
  hereOk = true;
  hereCmd = null;
  vi.clearAllMocks();
});

describe("playableOn", () => {
  it("yt anywhere, locals only on their owner, other nowhere", () => {
    expect(playableOn(ytRef("a"), PHONE)).toBe(true);
    expect(playableOn(localRef("/a", PHONE), PHONE)).toBe(true);
    expect(playableOn(localRef("/a", SELF), PHONE)).toBe(false);
    expect(playableOn({ src: "other", title: "x", artist: null, album: null, durationMs: null, artUrl: null }, PHONE)).toBe(false);
  });
});

describe("buildTransfer", () => {
  it("puts the current track first, with the position, then the queue", () => {
    const t = buildTransfer({ track: ytRef("c"), queue: [q("a"), q("b")], positionMs: 1234.4 }, PHONE)!;
    expect(t.cmd).toMatchObject({ type: "play", index: 0, positionMs: 1234 });
    expect(t.cmd.items.map((i) => (i as { videoId: string }).videoId)).toEqual(["c", "a", "b"]);
  });
  it("drops the position when the current track can't move", () => {
    const t = buildTransfer({ track: localRef("/x", SELF), queue: [q("a")], positionMs: 9_000 }, PHONE)!;
    expect(t.cmd).not.toHaveProperty("positionMs");
    expect(t).toMatchObject({ moved: 1, dropped: 1 });
  });
  it("omits a zero position", () => {
    expect(buildTransfer({ track: ytRef("c"), queue: [], positionMs: 0 }, PHONE)!.cmd).not.toHaveProperty("positionMs");
  });
  it("is null when nothing can move", () => {
    expect(buildTransfer({ track: localRef("/x", SELF), queue: [], positionMs: 0 }, PHONE)).toBeNull();
  });
});

describe("this computer -> other", () => {
  it("play, then pause here, then select — in that order", async () => {
    const r = await moveTo(PHONE, deps);
    expect(r).toEqual({ ok: true, moved: 3, dropped: 0 });
    expect(calls).toEqual(["cmd:phone:play", "pauseLocal", "select:phone"]);
    expect(hereCmd).toMatchObject({ type: "play", positionMs: 30_000 });
  });
  it("reports what could not move", async () => {
    local = body({ queue: [q("n1"), q("l", localRef("/l", SELF))] });
    await moveTo(PHONE, deps);
    expect(deps.info).toHaveBeenCalledWith(expect.stringContaining("1 track couldn't move"));
  });
  it("keeps playing here and says why when the phone refuses", async () => {
    cmdResult = { ok: false, error: "offline" };
    expect(await moveTo(PHONE, deps)).toEqual({ ok: false, reason: "failed" });
    expect(calls).toEqual(["cmd:phone:play"]);
    expect(deps.error).toHaveBeenCalledWith("Pixel is offline.");
  });
  it("does not switch when nothing playing can move", async () => {
    local = body({ track: localRef("/x", SELF), queue: [] });
    expect(await moveTo(PHONE, deps)).toEqual({ ok: false, reason: "nothing" });
    expect(calls).toEqual([]);
    expect(deps.error).toHaveBeenCalled();
  });
  it("hands over the role when paused on music that can't move", async () => {
    local = body({ track: localRef("/x", SELF), queue: [], playing: false });
    expect(await moveTo(PHONE, deps)).toEqual({ ok: true, moved: 0, dropped: 0 });
    expect(calls).toEqual(["select:phone"]);
    expect(deps.error).not.toHaveBeenCalled();
    expect(deps.info).toHaveBeenCalledWith(expect.stringContaining("Pixel is the player now"));
  });
  it("just selects the target when this computer is idle", async () => {
    local = body({ track: null, queue: [], playing: false, positionMs: 0 });
    expect(await moveTo(PHONE, deps)).toEqual({ ok: true, moved: 0, dropped: 0 });
    expect(calls).toEqual(["select:phone"]);
  });
});

describe("other -> this computer", () => {
  beforeEach(() => {
    source = PHONE;
    remote[PHONE] = stamped({ track: ytRef("pt"), queue: [q("p1")], positionMs: 10_000, at: 1_000, playing: true });
  });
  it("plays here from the interpolated position, pauses the phone, selects self", async () => {
    const r = await moveTo(SELF, deps);
    expect(r.ok).toBe(true);
    // 10 000 + (5 000 hub now - 1 000 at) while playing.
    expect(hereCmd).toMatchObject({ type: "play", index: 0, positionMs: 14_000 });
    expect(calls).toEqual(["playHere", "cmd:phone:pause", "select:me"]);
  });
  it("a paused phone does not advance the position", async () => {
    remote[PHONE] = stamped({ track: ytRef("pt"), queue: [], positionMs: 10_000, at: 1_000, playing: false });
    await moveTo(SELF, deps);
    expect(hereCmd).toMatchObject({ positionMs: 10_000 });
  });
  it("leaves the phone alone when starting here fails", async () => {
    hereOk = false;
    expect(await moveTo(SELF, deps)).toEqual({ ok: false, reason: "failed" });
    expect(calls).toEqual(["playHere"]);
  });
  it("reports the phone's own files that can't come along", async () => {
    remote[PHONE] = stamped({ track: ytRef("pt"), queue: [q("l", localRef("/l", PHONE))] });
    await moveTo(SELF, deps);
    expect(deps.info).toHaveBeenCalledWith(expect.stringContaining("1 track couldn't move"));
  });
  it("hands the role over when the phone's state is unknown", async () => {
    remote = {};
    await moveTo(SELF, deps);
    expect(calls).toEqual(["select:me"]);
  });
});

describe("other -> other", () => {
  beforeEach(() => {
    source = PHONE;
    remote[PHONE] = stamped({ track: ytRef("pt"), queue: [], positionMs: 5_000, at: 5_000 });
  });
  it("plays on the target, pauses the source, selects the target", async () => {
    await moveTo(TAB, deps);
    expect(calls).toEqual(["cmd:tab:play", "cmd:phone:pause", "select:tab"]);
  });
  it("can carry the target's own local files", async () => {
    remote[PHONE] = stamped({ track: ytRef("pt"), queue: [q("l", localRef("/l", TAB))] });
    await moveTo(TAB, deps);
    expect((hereCmd as { items: unknown[] }).items).toHaveLength(2);
  });
});

describe("same device", () => {
  it("does nothing when the target already plays", async () => {
    expect(await moveTo(SELF, deps)).toEqual({ ok: false, reason: "same" });
    source = PHONE;
    expect(await moveTo(PHONE, deps)).toEqual({ ok: false, reason: "same" });
    expect(calls).toEqual([]);
  });
});
