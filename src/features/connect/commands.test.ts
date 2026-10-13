import { describe, expect, it, vi } from "vitest";
import type { LibraryTrack } from "@/lib/types";
import type { QueueItem } from "@/stores/engine";
import { CommandQueue, executeCommand, type PlayerPort, type PlayerSnapshot } from "./commands";
import type { Command } from "./protocol";
import { QueueUidTracker } from "./queueUids";
import { DEVICE, libTrack, localRef, ytItem, ytRef } from "./testKit";

function fakePort(over: Partial<PlayerSnapshot> = {}, library: LibraryTrack[] = [libTrack("/a.mp3")]) {
  const snap: PlayerSnapshot = {
    playing: false,
    paused: false,
    seekable: true,
    queue: [],
    order: [],
    orderPos: -1,
    ...over,
  };
  const port = {
    deviceId: () => DEVICE,
    findLocal: vi.fn(async (path: string) => library.find((t) => t.path === path) ?? null),
    snapshot: () => snap,
    playItems: vi.fn(),
    playRadio: vi.fn(),
    startAt: vi.fn(),
    enqueue: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    jumpTo: vi.fn(),
    next: vi.fn(),
    prev: vi.fn(),
    seek: vi.fn(),
    stop: vi.fn(async () => {}),
    setVolume: vi.fn(),
    setShuffle: vi.fn(),
    setRepeat: vi.fn(),
  } satisfies PlayerPort;
  return { port, snap };
}

const run = (cmd: Command, port: PlayerPort, uids = new QueueUidTracker()) =>
  executeCommand(cmd, port, uids);

describe("play", () => {
  it("plays the requested index and passes only playable items", async () => {
    const { port } = fakePort();
    const out = await run(
      {
        type: "play",
        index: 2,
        items: [
          ytRef("a"),
          localRef("/elsewhere.mp3", "other-device"), // another device's file: skipped
          ytRef("c"),
          { src: "other", title: "x", artist: null, album: null, durationMs: null, artUrl: null },
          localRef("/a.mp3"),
        ],
      },
      port,
    );
    expect(out).toEqual({ ok: true });
    const [items, start] = port.playItems.mock.calls[0]! as [QueueItem[], number];
    expect(items.map((i) => i.id)).toEqual(["a", "c", "/a.mp3"]);
    expect(start).toBe(1); // "c" is still the item at the requested index
  });

  it("skips a local track that is in the library but belongs to another device", async () => {
    const { port } = fakePort({}, [libTrack("/a.mp3")]);
    const out = await run({ type: "play", index: 0, items: [localRef("/a.mp3", "other-device")] }, port);
    expect(out).toEqual({ ok: false, error: "nothing_playable" });
    expect(port.findLocal).not.toHaveBeenCalled();
  });

  it("falls to the next playable item when the requested one is skipped", async () => {
    const { port } = fakePort();
    await run(
      { type: "play", index: 1, items: [ytRef("a"), localRef("/nope.mp3"), ytRef("c")] },
      port,
    );
    const [items, start] = port.playItems.mock.calls[0]! as [QueueItem[], number];
    expect(items[start]!.id).toBe("c");
  });

  it("falls back to the closest earlier item when nothing follows", async () => {
    const { port } = fakePort();
    await run({ type: "play", index: 2, items: [ytRef("a"), ytRef("b"), localRef("/nope")] }, port);
    const [items, start] = port.playItems.mock.calls[0]! as [QueueItem[], number];
    expect(items[start]!.id).toBe("b");
  });

  it("acks nothing_playable and plays nothing when every item is skipped", async () => {
    const { port } = fakePort();
    const out = await run(
      { type: "play", index: 0, items: [localRef("/x", "other-device"), localRef("/missing")] },
      port,
    );
    expect(out).toEqual({ ok: false, error: "nothing_playable" });
    expect(port.playItems).not.toHaveBeenCalled();
    expect(port.playRadio).not.toHaveBeenCalled();
  });

  it("radio: seeds YT radio from the requested track instead of playing a list", async () => {
    const { port } = fakePort();
    await run({ type: "play", index: 0, radio: true, items: [ytRef("seed")] }, port);
    expect(port.playRadio).toHaveBeenCalledOnce();
    expect(port.playRadio.mock.calls[0]![0].videoId).toBe("seed");
    expect(port.playItems).not.toHaveBeenCalled();
  });

  it("radio on a local track just plays it", async () => {
    const { port } = fakePort();
    await run({ type: "play", index: 0, radio: true, items: [localRef("/a.mp3")] }, port);
    expect(port.playRadio).not.toHaveBeenCalled();
    expect(port.playItems).toHaveBeenCalledOnce();
  });

  it("starts at positionMs (in seconds) only when it is positive", async () => {
    const a = fakePort();
    await run({ type: "play", index: 0, items: [ytRef("a")], positionMs: 42_500 }, a.port);
    expect(a.port.startAt).toHaveBeenCalledWith(42.5);
    const b = fakePort();
    await run({ type: "play", index: 0, items: [ytRef("a")], positionMs: 0 }, b.port);
    await run({ type: "play", index: 0, items: [ytRef("a")] }, b.port);
    expect(b.port.startAt).not.toHaveBeenCalled();
  });

  it("a thrown failure becomes a failed ack, never an exception", async () => {
    const { port } = fakePort();
    port.playItems.mockImplementation(() => {
      throw new Error("boom");
    });
    expect(await run({ type: "play", index: 0, items: [ytRef("a")] }, port)).toEqual({
      ok: false,
      error: "failed",
    });
  });
});

describe("enqueue", () => {
  it("passes mode through and refuses an empty result", async () => {
    const { port } = fakePort();
    expect(await run({ type: "enqueue", mode: "next", items: [ytRef("a")] }, port)).toEqual({ ok: true });
    expect(port.enqueue.mock.calls[0]![1]).toBe("next");
    await run({ type: "enqueue", mode: "end", items: [ytRef("b")] }, port);
    expect(port.enqueue.mock.calls[1]![1]).toBe("end");
    expect(
      await run({ type: "enqueue", mode: "end", items: [localRef("/x", "other-device")] }, port),
    ).toEqual({ ok: false, error: "nothing_playable" });
    expect(port.enqueue).toHaveBeenCalledTimes(2);
  });
});

describe("pause / resume are explicit, never toggles", () => {
  it("pause pauses only a playing track", async () => {
    const playing = fakePort({ playing: true, paused: false });
    await run({ type: "pause" }, playing.port);
    expect(playing.port.pause).toHaveBeenCalledOnce();
    const paused = fakePort({ playing: true, paused: true });
    expect(await run({ type: "pause" }, paused.port)).toEqual({ ok: true });
    expect(paused.port.pause).not.toHaveBeenCalled();
    const idle = fakePort();
    await run({ type: "pause" }, idle.port);
    expect(idle.port.pause).not.toHaveBeenCalled();
  });

  it("resume un-pauses a paused track and leaves a playing one alone", async () => {
    const paused = fakePort({ playing: true, paused: true });
    await run({ type: "resume" }, paused.port);
    expect(paused.port.resume).toHaveBeenCalledOnce();
    const playing = fakePort({ playing: true, paused: false });
    expect(await run({ type: "resume" }, playing.port)).toEqual({ ok: true });
    expect(playing.port.resume).not.toHaveBeenCalled();
    expect(playing.port.jumpTo).not.toHaveBeenCalled();
  });

  it("resume when stopped replays the track the queue ended on", async () => {
    const { port } = fakePort({ playing: false, queue: [ytItem("a"), ytItem("b")], order: [0, 1], orderPos: 1 });
    expect(await run({ type: "resume" }, port)).toEqual({ ok: true });
    expect(port.jumpTo).toHaveBeenCalledWith(1);
  });

  it("resume with an empty queue is nothing_playable", async () => {
    const { port } = fakePort();
    expect(await run({ type: "resume" }, port)).toEqual({ ok: false, error: "nothing_playable" });
    expect(port.jumpTo).not.toHaveBeenCalled();
  });
});

describe("seek", () => {
  it("converts ms to seconds and clamps negatives to 0", async () => {
    const { port } = fakePort();
    await run({ type: "seek", positionMs: 90_250 }, port);
    expect(port.seek).toHaveBeenLastCalledWith(90.25);
    await run({ type: "seek", positionMs: -5 }, port);
    expect(port.seek).toHaveBeenLastCalledWith(0);
  });

  it("is unsupported on a source that cannot seek", async () => {
    const { port } = fakePort({ seekable: false });
    expect(await run({ type: "seek", positionMs: 1000 }, port)).toEqual({
      ok: false,
      error: "unsupported",
    });
    expect(port.seek).not.toHaveBeenCalled();
  });
});

describe("jump", () => {
  it("maps a uid to its play-order position", async () => {
    const queue = [ytItem("a"), ytItem("b"), ytItem("c")];
    const uids = new QueueUidTracker();
    const ids = uids.uidsFor(queue);
    // shuffled order: c, a, b — "a" (queue index 0) sits at order position 1
    const { port } = fakePort({ playing: true, queue, order: [2, 0, 1], orderPos: 0 });
    expect(await run({ type: "jump", uid: ids[0]! }, port, uids)).toEqual({ ok: true });
    expect(port.jumpTo).toHaveBeenCalledWith(1);
  });

  it("an unknown uid fails without jumping", async () => {
    const queue = [ytItem("a")];
    const { port } = fakePort({ playing: true, queue, order: [0], orderPos: 0 });
    expect(await run({ type: "jump", uid: "nope" }, port)).toEqual({ ok: false, error: "failed" });
    expect(port.jumpTo).not.toHaveBeenCalled();
  });
});

describe("simple transport", () => {
  it("volume is clamped to 0..1", async () => {
    const { port } = fakePort();
    await run({ type: "volume", value: 1.7 }, port);
    await run({ type: "volume", value: -0.3 }, port);
    await run({ type: "volume", value: 0.4 }, port);
    expect(port.setVolume.mock.calls.map((c) => c[0])).toEqual([1, 0, 0.4]);
  });

  it("shuffle, repeat, next, prev, stop reach the port", async () => {
    const { port } = fakePort();
    await run({ type: "shuffle", on: true }, port);
    await run({ type: "shuffle", on: false }, port);
    await run({ type: "repeat", mode: "one" }, port);
    await run({ type: "next" }, port);
    await run({ type: "prev" }, port);
    await run({ type: "stop" }, port);
    expect(port.setShuffle.mock.calls.map((c) => c[0])).toEqual([true, false]);
    expect(port.setRepeat).toHaveBeenCalledWith("one");
    expect(port.next).toHaveBeenCalledOnce();
    expect(port.prev).toHaveBeenCalledOnce();
    expect(port.stop).toHaveBeenCalledOnce();
  });
});

describe("CommandQueue", () => {
  it("runs tasks strictly in arrival order, even when an earlier one is slow", async () => {
    const q = new CommandQueue();
    const log: string[] = [];
    const slow = q.run(async () => {
      await new Promise((r) => setTimeout(r, 20));
      log.push("play");
    });
    const fast = q.run(async () => {
      log.push("pause");
    });
    await Promise.all([slow, fast]);
    expect(log).toEqual(["play", "pause"]);
  });

  it("keeps going after a task rejects", async () => {
    const q = new CommandQueue();
    const log: string[] = [];
    await q.run(async () => {
      throw new Error("x");
    }).catch(() => log.push("caught"));
    await q.run(async () => void log.push("next"));
    expect(log).toEqual(["caught", "next"]);
  });
});
