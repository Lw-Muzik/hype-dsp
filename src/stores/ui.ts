import { create } from "zustand";
import type { AppInfo, LicenseStatus } from "@/lib/types";

/** Top-level navigable views (one window, seven destinations). */
export type Route =
  | "enhancer"
  | "equalizer"
  | "mixer"
  | "stems"
  | "player"
  | "stations"
  | "visuals"
  | "settings";

/** Drag-resize bounds (px) for each sidebar: clamp + reset target. */
export const SIDEBAR_LIMITS = {
  left: { min: 200, max: 460, default: 240 },
  right: { min: 280, max: 600, default: 320 },
} as const;

type Side = "left" | "right";

const LS_KEY: Record<Side, string> = {
  left: "hm.leftSidebarWidth",
  right: "hm.rightSidebarWidth",
};

const clampWidth = (w: number, side: Side): number => {
  const { min, max } = SIDEBAR_LIMITS[side];
  return Math.min(max, Math.max(min, Math.round(w)));
};

const loadWidth = (side: Side): number => {
  try {
    const raw = localStorage.getItem(LS_KEY[side]);
    const n = raw != null ? Number(raw) : NaN;
    return Number.isFinite(n) ? clampWidth(n, side) : SIDEBAR_LIMITS[side].default;
  } catch {
    return SIDEBAR_LIMITS[side].default;
  }
};

const saveWidth = (side: Side, w: number): void => {
  try {
    localStorage.setItem(LS_KEY[side], String(w));
  } catch {
    // Private mode / no storage — width just won't persist.
  }
};

interface UiState {
  /** Currently displayed view. */
  route: Route;
  setRoute: (route: Route) => void;

  /** Whether the sidebar is collapsed to an icon rail (user toggle). */
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;

  /** Resizable sidebar widths (px), persisted across sessions. */
  leftWidth: number;
  rightWidth: number;
  setLeftWidth: (w: number) => void;
  setRightWidth: (w: number) => void;
  /** True while a separator is being dragged — sidebars drop their width
   *  transition so they track the cursor 1:1. */
  resizing: boolean;
  setResizing: (resizing: boolean) => void;

  /** The right sidebar's active tab, or null when hidden. */
  rightPanel: "queue" | "lyrics" | null;
  /** Open `tab` in the right sidebar, or close it if that tab is already open. */
  toggleRight: (tab: "queue" | "lyrics") => void;
  closeRight: () => void;

  /** App metadata, loaded once from the backend on startup. */
  appInfo: AppInfo | null;
  setAppInfo: (info: AppInfo) => void;

  /** Licensing status (mock). */
  license: LicenseStatus | null;
  setLicense: (license: LicenseStatus) => void;
}

export const useUiStore = create<UiState>((set) => ({
  route: "player",
  setRoute: (route) => set({ route }),

  sidebarCollapsed: false,
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),

  leftWidth: loadWidth("left"),
  rightWidth: loadWidth("right"),
  setLeftWidth: (w) => {
    const width = clampWidth(w, "left");
    saveWidth("left", width);
    set({ leftWidth: width });
  },
  setRightWidth: (w) => {
    const width = clampWidth(w, "right");
    saveWidth("right", width);
    set({ rightWidth: width });
  },
  resizing: false,
  setResizing: (resizing) => set({ resizing }),

  rightPanel: null,
  toggleRight: (tab) =>
    set((state) => ({ rightPanel: state.rightPanel === tab ? null : tab })),
  closeRight: () => set({ rightPanel: null }),

  appInfo: null,
  setAppInfo: (appInfo) => set({ appInfo }),

  license: null,
  setLicense: (license) => set({ license }),
}));
