// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";

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
import { snapshotSession } from "./playbackSession";
import {
  POSITION_SAVE_MS,
  SESSION_SAVE_THROTTLE_MS,
  useSessionPersistence,
} from "./useSessionPersistence";

function Probe() {
  useSessionPersistence();
  return null;
}

const store = () => useEngineStore.getState();
const item = (n: number) =>
  localItem({ path: `/m/${n}.mp3`, title: `T${n}`, artist: null, album: null, genre: null, durationSecs: 200 });
const saved = () =>
  vi.mocked(ipc.sessionSave).mock.calls.map(([json]) => (json ? JSON.parse(json) : null));
const tick = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};

const savedSession = JSON.stringify(
  snapshotSession({
    queue: [item(0), item(1)],
    order: [0, 1],
    orderPos: 1,
    shuffle: true,
    repeat: "one",
    positionSecs: 61,
    resumeAt: null,
  }),
);

beforeEach(async () => {
  vi.useFakeTimers();
  await store().stop();
  store().setPlaying(false);
  vi.clearAllMocks();
  useEngineStore.setState({
    shuffle: false,
    repeat: "off",
    state: { ...store().state, playback: { ...store().state.playback, gapless: false, crossfadeSecs: 0 } },
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("useSessionPersistence", () => {
  it("restores the saved session on launch, paused", async () => {
    vi.mocked(ipc.sessionLoad).mockResolvedValue(savedSession);
    render(<Probe />);
    await tick(0);
    expect(store().nowPlaying).toBe("T1");
    expect(store().shuffle).toBe(true);
    expect(store().repeat).toBe("one");
    expect(store().resumeAt).toBe(61);
    expect(store().paused).toBe(true);
  });

  it("never saves before the restore ran (it would erase the session)", async () => {
    let finish: (v: string | null) => void = () => {};
    vi.mocked(ipc.sessionLoad).mockReturnValue(new Promise((r) => (finish = r)));
    render(<Probe />);
    // The empty store churns while the load is pending.
    act(() => useEngineStore.setState({ repeat: "all" }));
    await tick(POSITION_SAVE_MS * 2);
    expect(ipc.sessionSave).not.toHaveBeenCalled();

    finish(savedSession);
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    // After restoring, the saved state is written back unchanged.
    expect(saved().at(-1)).toMatchObject({ orderPos: 1, positionSecs: 61, repeat: "one" });
  });

  it("saves queue changes, and the exact spot on pause", async () => {
    vi.mocked(ipc.sessionLoad).mockResolvedValue(null);
    render(<Probe />);
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    vi.mocked(ipc.sessionSave).mockClear();

    act(() => store().playFromList([item(5).track!, item(6).track!], 1));
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    expect(saved().at(-1)).toMatchObject({ orderPos: 1, positionSecs: 0 });

    act(() => useEngineStore.setState({ positionSecs: 33 }));
    act(() => store().togglePause());
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    expect(saved().at(-1)).toMatchObject({ positionSecs: 33 });
  });

  it("saves the position periodically only while playing, without duplicates", async () => {
    vi.mocked(ipc.sessionLoad).mockResolvedValue(null);
    render(<Probe />);
    await tick(0);
    act(() => store().playFromList([item(1).track!], 0));
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    vi.mocked(ipc.sessionSave).mockClear();

    act(() => useEngineStore.setState({ positionSecs: 12 }));
    await tick(POSITION_SAVE_MS);
    expect(saved().at(-1)).toMatchObject({ positionSecs: 12 });

    // Same position: nothing new to write.
    const before = vi.mocked(ipc.sessionSave).mock.calls.length;
    await tick(POSITION_SAVE_MS);
    expect(vi.mocked(ipc.sessionSave).mock.calls.length).toBe(before);

    // Pausing saves the spot once; after that the ticker stops writing.
    act(() => useEngineStore.setState({ paused: true, positionSecs: 50 }));
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    expect(saved().at(-1)).toMatchObject({ positionSecs: 50 });
    vi.mocked(ipc.sessionSave).mockClear();
    act(() => useEngineStore.setState({ positionSecs: 51 }));
    await tick(POSITION_SAVE_MS * 2);
    expect(ipc.sessionSave).not.toHaveBeenCalled();
  });

  it("an explicit stop clears the session", async () => {
    vi.mocked(ipc.sessionLoad).mockResolvedValue(savedSession);
    render(<Probe />);
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    await act(async () => {
      await store().stop();
    });
    await tick(SESSION_SAVE_THROTTLE_MS + 10);
    expect(vi.mocked(ipc.sessionSave).mock.calls.at(-1)?.[0]).toBeNull();
  });
});
