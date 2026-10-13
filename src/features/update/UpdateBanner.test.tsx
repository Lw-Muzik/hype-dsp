// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { UpdateBanner } from "./UpdateBanner";
import { UpdatePill } from "./UpdatePill";
import { useUpdateStore } from "@/stores/update";
import { useToastStore } from "@/stores/toast";

vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    updaterRestartNow: vi.fn(),
    updaterStatus: vi.fn(),
    onUpdaterStatus: vi.fn(),
  };
});

import { updaterRestartNow } from "@/lib/ipc";

function setReady(notes: string | null = null) {
  useUpdateStore.setState({
    status: { state: "ready", version: "0.1.30", notes },
    dismissedVersion: null,
    restarting: false,
  });
}

function Shell() {
  return (
    <>
      <UpdatePill />
      <UpdateBanner />
    </>
  );
}

beforeEach(() => {
  useToastStore.setState({ toasts: [] });
  vi.mocked(updaterRestartNow).mockReset();
});
afterEach(cleanup);

describe("<UpdateBanner>", () => {
  it("renders nothing when idle", () => {
    useUpdateStore.setState({ status: { state: "idle" }, dismissedVersion: null });
    const { container } = render(<Shell />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the version and a Restart button", () => {
    setReady();
    render(<Shell />);
    expect(screen.getByText("HypeMuzik 0.1.30 is ready")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart now" })).toBeInTheDocument();
  });

  it("collapses to the pill on Later and the pill restores the banner", () => {
    setReady();
    render(<Shell />);
    fireEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(screen.queryByText("HypeMuzik 0.1.30 is ready")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Update 0.1.30 ready/ }));
    expect(screen.getByText("HypeMuzik 0.1.30 is ready")).toBeInTheDocument();
  });

  it("offers What's new only when there are notes, as plain text", () => {
    setReady(null);
    const { unmount } = render(<Shell />);
    expect(screen.queryByRole("button", { name: "What's new" })).not.toBeInTheDocument();
    unmount();

    setReady("- fix <b>bold</b>\n- more");
    render(<Shell />);
    expect(screen.queryByText(/more/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "What's new" }));
    const notes = screen.getByText(/fix <b>bold<\/b>/);
    expect(notes.querySelector("b")).toBeNull();
  });

  it("toasts and re-enables when restart fails", async () => {
    setReady();
    vi.mocked(updaterRestartNow).mockRejectedValue(new Error("disk full"));
    render(<Shell />);
    fireEvent.click(screen.getByRole("button", { name: "Restart now" }));
    await waitFor(() => expect(useToastStore.getState().toasts).toHaveLength(1));
    expect(useToastStore.getState().toasts[0]?.kind).toBe("error");
    expect(screen.getByRole("button", { name: "Restart now" })).toBeEnabled();
  });

  it("shows Restarting… while the restart is in flight", () => {
    setReady();
    vi.mocked(updaterRestartNow).mockReturnValue(new Promise(() => {}));
    render(<Shell />);
    fireEvent.click(screen.getByRole("button", { name: "Restart now" }));
    expect(screen.getByRole("button", { name: "Restarting…" })).toBeDisabled();
  });
});

describe("<UpdatePill> downloading", () => {
  it("shows percent, or indeterminate without a total", () => {
    useUpdateStore.setState({
      status: { state: "downloading", version: "0.1.30", received: 45, total: 100 },
    });
    const { unmount } = render(<UpdatePill />);
    expect(screen.getByText("Updating… 45%")).toBeInTheDocument();
    unmount();
    useUpdateStore.setState({
      status: { state: "downloading", version: "0.1.30", received: 45, total: null },
    });
    render(<UpdatePill />);
    expect(screen.getByText("Updating…")).toBeInTheDocument();
  });
});
