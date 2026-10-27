// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { DeviceInfo } from "@/features/connect/protocol";
import { ytRef } from "@/features/connect/testKit";
import { configureRemote } from "@/features/connect/remote/control";
import type { RemoteView } from "@/features/connect/remote/session";
import { useRemoteStore } from "@/features/connect/remote/store";
import { useRemoteVolume } from "@/features/connect/remote/ui/useRemoteVolume";
import { useEngineStore } from "@/stores/engine";
import { TopBar } from "./TopBar";

vi.mock("@/lib/ipc", () => ({ openSettings: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@/features/player/ScrollingWaveform", () => ({ ScrollingWaveform: () => null }));
vi.mock("@/components/LevelMeter", () => ({ LevelMeter: () => null }));
vi.mock("@/components/PowerToggle", () => ({ PowerToggle: () => null }));
vi.mock("@/features/update/UpdatePill", () => ({ UpdatePill: () => null }));
vi.mock("@/features/mode/ModeSwitch", () => ({ ModeSwitch: () => null }));
vi.mock("@/features/notifications/NotificationBell", () => ({ NotificationBell: () => null }));

const PHONE: DeviceInfo = {
  id: "phone", kind: "mobile", name: "Pixel", platform: null, appVersion: null,
  caps: ["player", "yt"], online: true, lastSeenAt: "x",
};
const view = (over: Partial<RemoteView> = {}): RemoteView => ({
  device: PHONE, seq: 1, track: ytRef("a", { title: "Sauti" }), playing: true, positionMs: 0,
  at: Date.now(), durationMs: 200_000, volume: 0.4, shuffle: false, repeat: "off", queue: [], ...over,
});

const cmd = vi.fn().mockResolvedValue({ ok: true });
const setMasterVolume = vi.fn();

beforeEach(() => {
  cmd.mockClear();
  setMasterVolume.mockClear();
  configureRemote({ view: () => useRemoteStore.getState().view, cmd, hubNow: () => Date.now(), moveTo: vi.fn() });
  useRemoteStore.setState({ view: null });
  useEngineStore.setState({
    state: { ...useEngineStore.getState().state, masterVolume: 1.5 },
    setMasterVolume,
  });
});
afterEach(() => {
  cleanup();
  configureRemote(null);
  vi.useRealTimers();
});

describe("TopBar volume", () => {
  it("is the local master volume (0..2) outside remote mode", () => {
    render(<TopBar />);
    const slider = screen.getByRole("slider", { name: "Master volume" });
    expect(slider).toHaveAttribute("aria-valuemax", "2");
    expect(slider).toHaveAttribute("aria-valuenow", "1.5");
    expect(screen.getByText("150%")).toBeInTheDocument();
    fireEvent.keyDown(slider, { key: "ArrowRight" });
    expect(setMasterVolume).toHaveBeenCalled();
    expect(cmd).not.toHaveBeenCalled();
  });

  it("controls the remote device (0..1) in remote mode", () => {
    useRemoteStore.setState({ view: view() });
    render(<TopBar />);
    expect(screen.queryByRole("slider", { name: "Master volume" })).toBeNull();
    const slider = screen.getByRole("slider", { name: "Volume on Pixel" });
    expect(slider).toHaveAttribute("aria-valuemax", "1");
    expect(slider).toHaveAttribute("aria-valuenow", "0.4");
    expect(screen.getByText("40%")).toBeInTheDocument();
    expect(screen.getByTitle("Volume on Pixel")).toBeInTheDocument();
  });

  it("sends a volume command to the device, not the engine", () => {
    useRemoteStore.setState({ view: view() });
    render(<TopBar />);
    fireEvent.keyDown(screen.getByRole("slider", { name: "Volume on Pixel" }), { key: "ArrowRight" });
    expect(cmd).toHaveBeenCalledWith("phone", { type: "volume", value: expect.any(Number) });
    expect(cmd.mock.calls.at(-1)![1].value).toBeGreaterThan(0.4);
    expect(setMasterVolume).not.toHaveBeenCalled();
  });
});

describe("useRemoteVolume", () => {
  it("follows the drag, throttles sends to 120 ms and sends the final value on commit", () => {
    vi.useFakeTimers();
    useRemoteStore.setState({ view: view() });
    const { result } = renderHook(() => useRemoteVolume(0.4));
    act(() => result.current.onChange(0.5));
    act(() => result.current.onChange(0.55)); // inside the gap: local only
    expect(result.current.value).toBe(0.55);
    expect(cmd).toHaveBeenCalledTimes(1);
    act(() => void vi.advanceTimersByTime(130));
    act(() => result.current.onChange(0.6));
    expect(cmd).toHaveBeenCalledTimes(2);
    act(() => result.current.onCommit(0.7));
    expect(cmd).toHaveBeenLastCalledWith("phone", { type: "volume", value: 0.7 });
    expect(result.current.value).toBe(0.4); // drag released: back to the device's value
  });
});
