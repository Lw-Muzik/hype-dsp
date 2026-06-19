import { create } from "zustand";

/**
 * A tiny signal so views that render the library (the Player) re-fetch after it
 * changes elsewhere (a scan in Settings). Bumping `version` is all that's
 * needed — consumers depend on it in their load effect.
 */
interface LibraryStore {
  version: number;
  /** Mark the library as changed (e.g. after a scan), triggering re-fetches. */
  refresh: () => void;
}

export const useLibraryStore = create<LibraryStore>((set) => ({
  version: 0,
  refresh: () => set((s) => ({ version: s.version + 1 })),
}));
