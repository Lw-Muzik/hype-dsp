// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { DeviceInfo } from "../../protocol";
import { useConnectStore } from "../../store";
import { ytRef } from "../../testKit";
import { configureRemote } from "../control";
import type { RemoteView } from "../session";
import { useRemoteStore } from "../store";
import { DevicePicker } from "./DevicePicker";
import { RemoteBar } from "./RemoteBar";
import { RemoteGate } from "./RemoteNotice";
import { RemoteQueue } from "./RemoteQueue";

// jsdom has no layout, so the virtual list would render no rows.
vi.mock("@/components/VirtualList", () => ({
  VirtualList: ({ items, renderRow }: { items: unknown[]; renderRow: (i: unknown) => unknown }) => (
    <div>{items.map((i, n) => <div key={n}>{renderRow(i) as never}</div>)}</div>
  ),
}));

const dev = (id: string, name: string, kind: DeviceInfo["kind"] = "mobile"): DeviceInfo => ({
  id, kind, name, platform: null, appVersion: null, caps: ["player", "yt"], online: true, lastSeenAt: "x",
});
const ME = dev("me", "MacBook", "desktop");
const PHONE = dev("phone", "Pixel");

const view = (over: Partial<RemoteView> = {}): RemoteView => ({
  device: PHONE, seq: 1, track: ytRef("a", { title: "Sauti Ya Mama", artist: "Artist A" }),
  playing: true, positionMs: 0, at: Date.now(), durationMs: 200_000, volume: 0.5,
  shuffle: false, repeat: "off", queue: [{ uid: "q1", track: ytRef("b", { title: "Next One" }) }], ...over,
});

const cmd = vi.fn().mockResolvedValue({ ok: true });
const moveTo = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  cmd.mockClear();
  moveTo.mockClear();
  let current: RemoteView | null = null;
  configureRemote({ view: () => current, cmd, hubNow: () => Date.now(), moveTo });
  // expose the view the store holds to the control, like the hook does
  useRemoteStore.subscribe((s) => (current = s.view));
  useRemoteStore.setState({ view: null, players: [], activeId: null, selfId: "me" });
  useConnectStore.setState({ status: { enabled: true, deviceId: "me", connection: null, vizSubscribed: false } as never });
});
afterEach(() => {
  cleanup();
  configureRemote(null);
});

describe("RemoteBar", () => {
  it("shows the device's track and where it plays", () => {
    const v = view();
    useRemoteStore.setState({ view: v });
    render(<RemoteBar view={v} />);
    expect(screen.getByText("Sauti Ya Mama")).toBeInTheDocument();
    expect(screen.getByText("Artist A")).toBeInTheDocument();
    expect(screen.getByText(/Playing on Pixel/)).toBeInTheDocument();
  });
  it("says paused when it is", () => {
    const v = view({ playing: false });
    useRemoteStore.setState({ view: v });
    render(<RemoteBar view={v} />);
    expect(screen.getByText(/Paused on Pixel/)).toBeInTheDocument();
  });
  it("sends transport commands to the device", () => {
    const v = view();
    useRemoteStore.setState({ view: v });
    render(<RemoteBar view={v} />);
    fireEvent.click(screen.getByLabelText("Pause"));
    fireEvent.click(screen.getByLabelText("Next track"));
    fireEvent.click(screen.getByLabelText("Previous track"));
    fireEvent.click(screen.getByLabelText("Shuffle"));
    fireEvent.click(screen.getByLabelText("Repeat off"));
    expect(cmd.mock.calls.map((c) => [c[0], c[1]])).toEqual([
      ["phone", { type: "pause" }],
      ["phone", { type: "next" }],
      ["phone", { type: "prev" }],
      ["phone", { type: "shuffle", on: true }],
      ["phone", { type: "repeat", mode: "all" }],
    ]);
  });
  it("resumes when paused", () => {
    const v = view({ playing: false });
    useRemoteStore.setState({ view: v });
    render(<RemoteBar view={v} />);
    fireEvent.click(screen.getByLabelText("Play"));
    expect(cmd).toHaveBeenCalledWith("phone", { type: "resume" });
  });
  it("disables transport with nothing playing", () => {
    const v = view({ track: null, durationMs: null, playing: false });
    useRemoteStore.setState({ view: v });
    render(<RemoteBar view={v} />);
    expect(screen.getByLabelText("Play")).toBeDisabled();
    expect(screen.getByText("Nothing playing")).toBeInTheDocument();
  });
});

describe("RemoteQueue", () => {
  it("lists up next and jumps on tap", () => {
    useRemoteStore.setState({ view: view() });
    render(<RemoteQueue view={view()} />);
    expect(screen.getByText("Sauti Ya Mama")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Next One"));
    expect(cmd).toHaveBeenCalledWith("phone", { type: "jump", uid: "q1" });
  });
  it("says so when nothing is queued", () => {
    render(<RemoteQueue view={view({ queue: [] })} />);
    expect(screen.getByText("Nothing up next.")).toBeInTheDocument();
  });
});

describe("DevicePicker", () => {
  it("is hidden when no other player is online", () => {
    useRemoteStore.setState({ players: [ME] });
    const { container } = render(<DevicePicker />);
    expect(container).toBeEmptyDOMElement();
  });
  it("is hidden when Hype Connect is off", () => {
    useConnectStore.setState({ status: { enabled: false } as never });
    useRemoteStore.setState({ players: [ME, PHONE] });
    const { container } = render(<DevicePicker />);
    expect(container).toBeEmptyDOMElement();
  });
  it("lists this computer first, marks the one playing, and moves playback on tap", () => {
    useRemoteStore.setState({ players: [PHONE, ME], activeId: "phone" });
    render(<DevicePicker />);
    fireEvent.click(screen.getByLabelText("Play on a device"));
    const items = screen.getAllByRole("menuitem");
    expect(items[0]).toHaveTextContent("This computer");
    expect(items[1]).toHaveTextContent("Pixel");
    expect(items[1]).toHaveTextContent("Playing now");
    fireEvent.click(items[0]!);
    expect(moveTo).toHaveBeenCalledWith("me");
  });
  it("tapping the device already playing does nothing", () => {
    useRemoteStore.setState({ players: [ME, PHONE], activeId: "phone" });
    render(<DevicePicker />);
    fireEvent.click(screen.getByLabelText("Play on a device"));
    fireEvent.click(screen.getAllByRole("menuitem")[1]!);
    expect(moveTo).not.toHaveBeenCalled();
  });
  it("marks this computer when nobody is active", () => {
    useRemoteStore.setState({ players: [ME, PHONE], activeId: null });
    render(<DevicePicker />);
    fireEvent.click(screen.getByLabelText("Play on a device"));
    expect(screen.getAllByRole("menuitem")[0]).toHaveTextContent("Playing now");
  });
});

describe("RemoteGate", () => {
  it("shows the children locally and the notice while remote", () => {
    const { rerender } = render(<RemoteGate what="Lyrics"><p>local lyrics</p></RemoteGate>);
    expect(screen.getByText("local lyrics")).toBeInTheDocument();
    useRemoteStore.setState({ view: view() });
    rerender(<RemoteGate what="Lyrics"><p>local lyrics</p></RemoteGate>);
    expect(screen.queryByText("local lyrics")).toBeNull();
    expect(screen.getByText("Lyrics are on Pixel")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Play on this computer"));
    expect(moveTo).toHaveBeenCalledWith("me");
  });
  it("uses 'is' for a singular subject", () => {
    useRemoteStore.setState({ view: view() });
    render(<RemoteGate what="The visualiser"><p /></RemoteGate>);
    expect(screen.getByText("The visualiser is on Pixel")).toBeInTheDocument();
  });
});
