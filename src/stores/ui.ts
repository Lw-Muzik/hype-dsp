import { create } from "zustand";
import type { AppInfo } from "@/lib/types";

/** Top-level navigable views (one window, six destinations). */
export type Route =
  | "enhancer"
  | "equalizer"
  | "mixer"
  | "player"
  | "radio"
  | "settings";

interface UiState {
  /** Currently displayed view. */
  route: Route;
  setRoute: (route: Route) => void;

  /** Whether the sidebar is collapsed to an icon rail (user toggle). */
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;

  /** App metadata, loaded once from the backend on startup. */
  appInfo: AppInfo | null;
  setAppInfo: (info: AppInfo) => void;
}

export const useUiStore = create<UiState>((set) => ({
  route: "enhancer",
  setRoute: (route) => set({ route }),

  sidebarCollapsed: false,
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),

  appInfo: null,
  setAppInfo: (appInfo) => set({ appInfo }),
}));
