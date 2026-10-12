import { beforeEach, describe, expect, it, vi } from "vitest";

// Every IPC call becomes a recorded no-op that resolves; the helper that
// formats errors stays real.
vi.mock("@/lib/ipc", async (importOriginal) => {
  const real = await importOriginal<Record<string, unknown>>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [name, value] of Object.entries(real)) {
    if (typeof value === "function" && name !== "ipcErrorMessage") {
      mocked[name] = vi.fn(() => Promise.resolve(undefined));
    }
  }
  return mocked;
});

import * as ipc from "@/lib/ipc";
import { localItem, useEngineStore } from "@/stores/engine";
import type { LibraryTrack } from "@/lib/types";

const track = (path: string): LibraryTrack => ({
  path,
  title: path,
  artist: null,
  album: null,
  genre: null,
  durationSecs: 100,
});
const item = (path: string) => localItem(track(path));
const store = () => useEngineStore.getState();
const playQueue = () => vi.mocked(ipc.playerPlayQueue);
const paths = () => store().order.map((i) => store().queue[i]!.id);

beforeEach(async () => {
  await store().stop();
  // stop() latches "user stopped" until the engine reports the stop; the mocked
  // engine never does, so deliver it here.
  store().setPlaying(false);
  vi.clearAllMocks();
  useEngineStore.setState({
    shuffle: false,
    repeat: "off",
    state: { ...store().state, playback: { ...store().state.playback, gapless: true } },
  });
});

describe("enqueueItems", () => {
  it("does nothing for an empty list", () => {
    store().enqueueItems([], "next");
    expect(store().queue).toHaveLength(0);
    expect(playQueue()).not.toHaveBeenCalled();
  });

  it("while idle, starts playback with the items", () => {
    store().enqueueItems([item("/a"), item("/b")], "end");
    expect(store().playing).toBe(true);
    expect(paths()).toEqual(["/a", "/b"]);
    expect(playQueue()).toHaveBeenCalledWith(["/a", "/b"], 0);
  });

  it("'end' appends in order without touching the engine's playing list", () => {
    store().playQueueItems([item("/a"), item("/b")], 0);
    playQueue().mockClear();
    store().enqueueItems([item("/c")], "end");
    expect(paths()).toEqual(["/a", "/b", "/c"]);
    expect(store().orderPos).toBe(0);
    expect(playQueue()).not.toHaveBeenCalled();
  });

  it("'next' slots right after the current track in play order", () => {
    store().playQueueItems([item("/a"), item("/b"), item("/c")], 1);
    playQueue().mockClear();
    store().enqueueItems([item("/x"), item("/y")], "next");
    expect(paths()).toEqual(["/a", "/b", "/x", "/y", "/c"]);
    expect(store().orderPos).toBe(1);
    expect(store().nowPlaying).toBe("/b");
    expect(playQueue()).not.toHaveBeenCalled(); // current track is not restarted
  });

  it("'next' during the engine's gapless list: at the next track change the store re-issues playback from its own order", () => {
    store().playQueueItems([item("/a"), item("/b"), item("/c")], 0);
    expect(playQueue()).toHaveBeenLastCalledWith(["/a", "/b", "/c"], 0);
    store().enqueueItems([item("/x")], "next");
    playQueue().mockClear();

    // The engine moves on through ITS old list (to /b, absolute index 1).
    store().applyQueueIndex(1);
    expect(playQueue()).toHaveBeenCalledOnce();
    // /x plays next, from the store's order, with the engine handed the new list.
    expect(playQueue()).toHaveBeenCalledWith(["/a", "/x", "/b", "/c"], 1);
    expect(store().nowPlaying).toBe("/x");
  });

  it("'next' when the engine stops on the current track: continues with the inserted item, not the rest of the list", () => {
    store().playQueueItems([item("/a"), item("/b")], 0);
    store().enqueueItems([item("/x")], "next");
    playQueue().mockClear();
    store().setPlaying(false); // the engine ended without reporting a track change
    expect(store().nowPlaying).toBe("/x");
    expect(playQueue()).toHaveBeenCalledWith(["/a", "/x", "/b"], 1);
  });

  it("without a pending insertion the engine's own track change is followed, not re-issued", () => {
    store().playQueueItems([item("/a"), item("/b"), item("/c")], 0);
    playQueue().mockClear();
    store().applyQueueIndex(1);
    expect(playQueue()).not.toHaveBeenCalled();
    expect(store().orderPos).toBe(1);
    expect(store().nowPlaying).toBe("/b");
  });
});

describe("setShuffle / setRepeat", () => {
  it("are explicit: setting the same value twice is stable", () => {
    store().setShuffle(true);
    store().setShuffle(true);
    expect(store().shuffle).toBe(true);
    store().setShuffle(false);
    expect(store().shuffle).toBe(false);
    store().setRepeat("one");
    store().setRepeat("one");
    expect(store().repeat).toBe("one");
  });

  it("shuffling keeps the current track current", () => {
    store().playQueueItems([item("/a"), item("/b"), item("/c"), item("/d")], 2);
    store().setShuffle(true);
    expect(store().queue[store().queueIndex]!.id).toBe("/c");
    expect(store().order[store().orderPos]).toBe(store().queueIndex);
  });
});
