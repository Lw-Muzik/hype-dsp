import { create } from "zustand";
import type { SettingsSection } from "./sections";

interface SettingsDialogState {
  open: boolean;
  /** Section the last request pointed at; null opens at the top. */
  section: SettingsSection | null;
  /** Bumped on every request, so asking again for the same section while the
   *  dialog is open still scrolls to it. */
  requestId: number;
}

export const useSettingsDialogStore = create<SettingsDialogState>(() => ({
  open: false,
  section: null,
  requestId: 0,
}));

/** Open Settings, optionally scrolled to a section. Resolves immediately; it
 *  returns a Promise so call sites written for the old IPC version still compile. */
export function openSettings(section?: SettingsSection): Promise<void> {
  useSettingsDialogStore.setState((s) => ({
    open: true,
    section: section ?? null,
    requestId: s.requestId + 1,
  }));
  return Promise.resolve();
}

export function closeSettings(): void {
  useSettingsDialogStore.setState({ open: false, section: null });
}
