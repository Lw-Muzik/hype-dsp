import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
import type { PlaybackSession } from "@/features/session/playbackSession";
import type { TransportProgress } from "@/lib/types";

const store = () => useEngineStore.getState();
const item = (n: number) =>
  localItem({ path: `/m/${n}.mp3`, title: `T${n}`, artist: null, album: null, genre: null, durationSecs: 200 });

const session = (over: Partial<PlaybackSession> = {}): PlaybackSession => ({
  v: 1,
  queue: [item(0), item(1), item(2)],
  order: [0, 1, 2],
  orderPos: 1,
  shuffle: false,
  repeat: "all",
  positionSecs: 95,
  ...over,
});

const progress = (over: Partial<TransportProgress> = {}): TransportProgress => ({
  positionSecs: 0,
  durationSecs: 200,
  paused: false,
  seekable: true,
  buffering: false,
  downloadBps: 0,
  rebufferCount: 0,
  ...over,
});

beforeEach(async () => {
  await store().stop();
  store().setPlaying(false);
  vi.clearAllMocks();
  // Single-file playback (no gapless queue) keeps the assertions simple.
  useEngineStore.setState({
    shuffle: false,
    repeat: "off",
    state: { ...store().state, playback: { ...store().state.playback, gapless: false, crossfadeSecs: 0 } },
  });
});
afterEach(() => {
  vi.useRealTimers();
});

describe("restoring a session", () => {
  it("brings the queue back paused, loading nothing", () => {
    expect(store().restoreSession(session())).toBe(true);
    const s = store();
    expect(s.queue.map((q) => q.id)).toEqual(["/m/0.mp3", "/m/1.mp3", "/m/2.mp3"]);
    expect(s.orderPos).toBe(1);
    expect(s.repeat).toBe("all");
    expect(s.nowPlaying).toBe("T1");
    expect(s.playing).toBe(false);
    expect(s.paused).toBe(true);
    expect(s.resumeAt).toBe(95);
    expect(s.positionSecs).toBe(95);
    expect(ipc.playerPlayFile).not.toHaveBeenCalled();
    expect(ipc.playerPlayQueue).not.toHaveBeenCalled();
  });

  it("doesn't replace something already queued (e.g. files opened at launch)", () => {
    store().playFromList([item(9).track!], 0);
    vi.clearAllMocks();
    expect(store().restoreSession(session())).toBe(false);
    expect(store().nowPlaying).toBe("T9");
  });
});

describe("resuming a restored session", () => {
  it("play loads the current track, then seeks to the saved spot once it can", () => {
    store().restoreSession(session());
    store().togglePause();

    expect(ipc.playerPlayFile).toHaveBeenCalledWith("/m/1.mp3");
    expect(store().resumeAt).toBeNull();
    expect(store().playing).toBe(true);

    // Not seekable yet (a stream still opening): wait.
    store().applyProgress(progress({ seekable: false }));
    expect(ipc.playerSeek).not.toHaveBeenCalled();

    store().applyProgress(progress({ seekable: true }));
    expect(ipc.playerSeek).toHaveBeenCalledWith(95);
    expect(store().positionSecs).toBe(95);

    // Only once.
    store().applyProgress(progress({ positionSecs: 96 }));
    expect(ipc.playerSeek).toHaveBeenCalledTimes(1);
  });

  it("the media-key / tray / remote 'play' resumes it too", () => {
    store().restoreSession(session());
    store().handleMediaCommand("play", null);
    expect(ipc.playerPlayFile).toHaveBeenCalledWith("/m/1.mp3");
  });

  it("scrubbing a restored track moves where it resumes, without loading it", () => {
    store().restoreSession(session());
    store().seek(30);
    expect(store().resumeAt).toBe(30);
    expect(ipc.playerSeek).not.toHaveBeenCalled();
    store().togglePause();
    store().applyProgress(progress());
    expect(ipc.playerSeek).toHaveBeenCalledWith(30);
  });

  it("doesn't seek into a track's last seconds", () => {
    store().restoreSession(session({ positionSecs: 199 }));
    store().togglePause();
    store().applyProgress(progress({ durationSecs: 200 }));
    expect(ipc.playerSeek).not.toHaveBeenCalled();
  });

  it("choosing another track drops the saved spot", () => {
    store().restoreSession(session());
    store().next();
    expect(ipc.playerPlayFile).toHaveBeenCalledWith("/m/2.mp3");
    expect(store().resumeAt).toBeNull();
    store().applyProgress(progress());
    expect(ipc.playerSeek).not.toHaveBeenCalled();
  });

  it("skipping before the resumed track could seek doesn't drag the saved spot to the next one", () => {
    store().restoreSession(session());
    store().togglePause(); // seek to 95 armed, waiting for "seekable"
    store().next(); // the user moves on first
    expect(ipc.playerPlayFile).toHaveBeenLastCalledWith("/m/2.mp3");
    store().applyProgress(progress());
    expect(ipc.playerSeek).not.toHaveBeenCalled();
  });

  it("stop and an incoming phone cast drop it too", async () => {
    store().restoreSession(session());
    store().togglePause();
    await store().stop();
    store().setPlaying(false);
    store().applyProgress(progress());
    expect(ipc.playerSeek).not.toHaveBeenCalled();

    store().restoreSession(session());
    store().togglePause();
    store().castIncoming("Phone song", null);
    store().applyProgress(progress());
    expect(ipc.playerSeek).not.toHaveBeenCalled();
    expect(store().resumeAt).toBeNull();
  });

  it("gives up on the seek if the track never becomes seekable", () => {
    vi.useFakeTimers();
    store().restoreSession(session());
    store().togglePause();
    store().applyProgress(progress({ seekable: false }));
    vi.advanceTimersByTime(31_000);
    store().applyProgress(progress({ seekable: true }));
    expect(ipc.playerSeek).not.toHaveBeenCalled();
  });
});
