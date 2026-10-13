// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    systemEqStatus: vi.fn(async () => "active"),
    systemAudioStatus: vi.fn(async () => ({ supported: true })),
    mixerListSessions: vi.fn(async () => ({ sessions: [] })),
    linkDiscoverStart: vi.fn(async () => {}),
    onPairedOnline: vi.fn(async () => () => {}),
    onRemoteConnected: vi.fn(async () => () => {}),
    openSettings: vi.fn(async () => {}),
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/features/theme/ThemeBackdrop", () => ({ default: () => null }));
vi.mock("@/components/TopBar", () => ({ TopBar: () => null }));
vi.mock("@/components/Toaster", () => ({ Toaster: () => null }));
vi.mock("@/components/PremiumGate", () => ({
  SupporterUpsellModal: () => null,
  requirePremium: () => true,
  PremiumGate: ({ children }: { children: unknown }) => children,
}));
vi.mock("@/features/update/UpdateBanner", () => ({ UpdateBanner: () => null }));
vi.mock("@/components/NowPlayingBar", () => ({ NowPlayingBar: () => <div data-testid="npbar" /> }));
vi.mock("@/components/RightSidebar", () => ({ RightSidebar: () => <div data-testid="rsb" /> }));
vi.mock("@/components/ResizeHandle", () => ({ ResizeHandle: () => null }));
vi.mock("@/components/SidebarNowPlaying", () => ({
  SidebarNowPlaying: () => <div data-testid="snp" />,
}));
vi.mock("@/app/router", () => ({ Router: () => null }));
vi.mock("@/features/tray/useTray", () => ({ useTray: () => {} }));
vi.mock("@/stores/musicLibrary", () => ({
  useMusicLibraryStore: Object.assign(
    (sel: (s: unknown) => unknown) => sel({ revalidateLocal: () => {} }),
    { getState: () => ({ syncPhone: () => {} }) },
  ),
}));
vi.mock("@/stores/update", () => ({
  useUpdateStore: { getState: () => ({ init: () => {} }) },
}));

import { App } from "@/app/App";
import { useSystemEqStore } from "@/stores/systemEq";
import { useUiStore } from "@/stores/ui";

const navLabels = () =>
  Array.from(screen.getByRole("navigation", { name: "Primary" }).querySelectorAll("button")).map(
    (b) => b.textContent?.trim(),
  );

beforeEach(() => {
  useSystemEqStore.setState({ enabled: false, resumeAttempted: true });
  useUiStore.setState({ route: "player" });
});
afterEach(cleanup);

describe("shell modes", () => {
  it("Player mode shows everything as before", () => {
    render(<App />);
    expect(navLabels()).toEqual([
      "Library", "Enhancer", "Equalizer", "Mixer", "Stems", "Explore", "Stations", "Movies", "Visuals",
    ]);
    expect(screen.getByTestId("npbar")).toBeInTheDocument();
    expect(screen.getByTestId("rsb")).toBeInTheDocument();
    expect(screen.getByTestId("snp")).toBeInTheDocument();
  });

  it("Equalizer mode shows only the four tuning views and no player chrome", async () => {
    useSystemEqStore.setState({ enabled: true });
    useUiStore.setState({ route: "equalizer" });
    render(<App />);
    expect(navLabels()).toEqual(["Equalizer", "Enhancer", "Stems", "Visuals"]);
    expect(screen.queryByTestId("npbar")).toBeNull();
    expect(screen.queryByTestId("rsb")).toBeNull();
    expect(screen.queryByTestId("snp")).toBeNull();
    await waitFor(() => expect(screen.getByText("Equalizing system audio")).toBeInTheDocument());
  });

  it("a route hidden in Equalizer mode falls back to equalizer", async () => {
    useSystemEqStore.setState({ enabled: true });
    useUiStore.setState({ route: "mixer" });
    render(<App />);
    await waitFor(() => expect(useUiStore.getState().route).toBe("equalizer"));
  });
});
