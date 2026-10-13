// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, act } from "@testing-library/react";
import { StrictMode } from "react";

vi.mock("@/lib/ipc", async (orig) => ({
  ...(await orig<typeof import("@/lib/ipc")>()),
  trayUpdate: vi.fn(async () => {}),
  onTrayAction: vi.fn(),
  systemAudioStatus: vi.fn(async () => ({ supported: true })),
  eqListPresets: vi.fn(async () => []),
  visualizerAvailable: vi.fn(async () => false),
}));
vi.mock("./trayActions", () => ({ runTrayAction: vi.fn(async () => {}) }));

import { eqListPresets, onTrayAction, trayUpdate } from "@/lib/ipc";
import { useEngineStore } from "@/stores/engine";
import { runTrayAction } from "./trayActions";
import { useTray } from "./useTray";

function Probe() {
  useTray();
  return null;
}

let handler: ((a: string) => void) | undefined;
const unlisten = vi.fn();
const flush = async () => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(200);
  });
};

beforeEach(() => {
  vi.useFakeTimers();
  handler = undefined;
  vi.mocked(onTrayAction).mockImplementation(async (h) => {
    handler = h;
    return unlisten;
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("useTray", () => {
  it("pushes the initial model once, then only on change (debounced)", async () => {
    render(<Probe />);
    await flush();
    expect(trayUpdate).toHaveBeenCalledTimes(1);

    // Unrelated store churn that leaves the model identical: no push.
    act(() => useEngineStore.setState({ queue: [] }));
    await flush();
    expect(trayUpdate).toHaveBeenCalledTimes(1);

    // Several rapid changes coalesce into one push.
    act(() => useEngineStore.setState({ nowPlaying: "A", playing: true }));
    act(() => useEngineStore.setState({ nowPlaying: "B", playing: true }));
    await flush();
    expect(trayUpdate).toHaveBeenCalledTimes(2);
    expect(vi.mocked(trayUpdate).mock.calls[1]![0]).toMatchObject({ nowPlaying: "B", playing: true });
    act(() => useEngineStore.setState({ nowPlaying: null, playing: false }));
    await flush();
  });

  it("a track change reaches the tray while meter frames stream in", async () => {
    render(<Probe />);
    await flush();
    vi.mocked(trayUpdate).mockClear();

    // Playback: the engine store changes every ~30 ms (meters). A debounce that
    // restarts on every change would never fire.
    act(() => useEngineStore.setState({ nowPlaying: "Live track", playing: true }));
    for (let i = 0; i < 20; i++) {
      act(() => useEngineStore.setState({ meters: { ...useEngineStore.getState().meters } }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30);
      });
    }
    expect(trayUpdate).toHaveBeenCalledTimes(1);
    expect(vi.mocked(trayUpdate).mock.calls[0]![0]).toMatchObject({ nowPlaying: "Live track" });
    act(() => useEngineStore.setState({ nowPlaying: null, playing: false }));
    await flush();
  });

  it("an action runs runTrayAction then force-pushes an unchanged model", async () => {
    render(<Probe />);
    await flush();
    const before = vi.mocked(trayUpdate).mock.calls.length;
    await act(async () => {
      handler!("fx:bass");
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(runTrayAction).toHaveBeenCalledWith("fx:bass");
    expect(vi.mocked(trayUpdate).mock.calls.length).toBe(before + 1);
  });

  it("reloads presets when the active preset changes", async () => {
    render(<Probe />);
    await flush();
    const n = vi.mocked(eqListPresets).mock.calls.length;
    act(() =>
      useEngineStore.setState((s) => ({ state: { ...s.state, activePresetId: "x" } })),
    );
    await flush();
    expect(vi.mocked(eqListPresets).mock.calls.length).toBe(n + 1);
  });

  it("cleans up under StrictMode and swallows IPC failures", async () => {
    vi.mocked(trayUpdate).mockRejectedValue(new Error("no tray"));
    const { unmount } = render(
      <StrictMode>
        <Probe />
      </StrictMode>,
    );
    await flush();
    unmount();
    await flush();
    expect(unlisten).toHaveBeenCalled();
    const calls = vi.mocked(trayUpdate).mock.calls.length;
    act(() => useEngineStore.setState({ nowPlaying: "Z" }));
    await flush();
    expect(vi.mocked(trayUpdate).mock.calls.length).toBe(calls);
  });
});
