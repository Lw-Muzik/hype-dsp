// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { AppNotification } from "@/lib/ipc";

const ipc = vi.hoisted(() => ({
  notificationsList: vi.fn(),
  notificationsOpen: vi.fn(),
  notificationsMarkAllRead: vi.fn(),
  onNotificationsUpdated: vi.fn(),
}));
vi.mock("@/lib/ipc", () => ipc);

import { NotificationBell, recent } from "./NotificationBell";

const iso = (daysAgo: number) => new Date(Date.now() - daysAgo * 86_400_000).toISOString();
const n = (id: string, over: Partial<AppNotification> = {}): AppNotification => ({
  id, title: `Title ${id}`, body: `Body ${id}`, link: null, createdAt: iso(1), read: false, ...over,
});

let pushUpdate: () => void;

beforeEach(() => {
  Object.values(ipc).forEach((f) => f.mockReset());
  ipc.onNotificationsUpdated.mockImplementation((h: () => void) => {
    pushUpdate = h;
    return Promise.resolve(() => {});
  });
});
afterEach(cleanup);

describe("NotificationBell", () => {
  it("shows an unread dot only while something is unread", async () => {
    ipc.notificationsList.mockResolvedValue([n("a"), n("b", { read: true })]);
    render(<NotificationBell />);
    expect(await screen.findByTestId("unread-dot")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Notifications, 1 unread" })).toBeInTheDocument();
  });

  it("has no dot when everything is read or older than 14 days", async () => {
    ipc.notificationsList.mockResolvedValue([n("a", { read: true }), n("old", { createdAt: iso(20) })]);
    render(<NotificationBell />);
    await waitFor(() => expect(ipc.notificationsList).toHaveBeenCalled());
    expect(screen.queryByTestId("unread-dot")).toBeNull();
  });

  it("refreshes live when the poller emits", async () => {
    ipc.notificationsList.mockResolvedValueOnce([]).mockResolvedValue([n("a")]);
    render(<NotificationBell />);
    await waitFor(() => expect(ipc.onNotificationsUpdated).toHaveBeenCalled());
    expect(screen.queryByTestId("unread-dot")).toBeNull();
    pushUpdate();
    expect(await screen.findByTestId("unread-dot")).toBeInTheDocument();
  });

  it("opening an item reports it via the open command and clears its dot", async () => {
    ipc.notificationsList.mockResolvedValue([n("a", { link: "https://hypemuzik.com/news" })]);
    ipc.notificationsOpen.mockResolvedValue([n("a", { link: "https://hypemuzik.com/news", read: true })]);
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: /Notifications, 1 unread/ }));
    fireEvent.click(screen.getByText("Title a"));
    // The command marks read, posts `opened` and opens the link natively.
    expect(ipc.notificationsOpen).toHaveBeenCalledWith("a");
    await waitFor(() => expect(screen.queryByTestId("unread-dot")).toBeNull());
  });

  it("lists only the last 14 days, newest first", () => {
    const out = recent([n("old", { createdAt: iso(15) }), n("x", { createdAt: iso(3) }), n("y", { createdAt: iso(1) })]);
    expect(out.map((i) => i.id)).toEqual(["y", "x"]);
  });

  it("shows an empty state", async () => {
    ipc.notificationsList.mockResolvedValue([]);
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications" }));
    expect(screen.getByText(/Nothing new/)).toBeInTheDocument();
  });
});
