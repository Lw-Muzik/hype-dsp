import { create } from "zustand";
import {
  ipcErrorMessage,
  ytmusicExploreCategories,
  ytmusicExplorePage,
  ytmusicExploreTracks,
  ytmusicStatus,
} from "@/lib/ipc";
import type { ExploreCategory, ExploreItem, ExploreSection, ExploreShelf } from "@/lib/types";
import type { LoadStatus } from "@/stores/musicLibrary";
import { ytmusicItem, useEngineStore } from "@/stores/engine";

/**
 * Explore — YouTube's own catalog, browsed lazily.
 *
 * Deliberately NOT part of `musicLibrary`. That store is eager: it walks every
 * source up front, merges the tracks into one list and caches them to disk,
 * which is right for music you *have*. Explore is music you *don't*: 44
 * categories holding ~7,000 playlists and ~1,200 albums, and its whole value is
 * being current. So nothing here is cached or merged — each open is a fresh
 * read, and a category page is fetched only when you click it.
 */

/** Only one category page is ever in flight; a newer click cancels an older. */
let pageGen = 0;

interface ExploreStore {
  /** Explore needs a session. Settled by the status call, never inferred from a
   *  failed fetch — conflating the two is what made the Library claim you were
   *  signed out whenever a listing broke. */
  signedIn: boolean;
  sections: ExploreSection[];
  sectionsLoad: LoadStatus;
  sectionsError: string | null;

  /** The category being viewed; null = the category picker. */
  selected: ExploreCategory | null;
  shelves: ExploreShelf[];
  pageLoad: LoadStatus;
  pageError: string | null;

  /** Id of the item being opened, so its tile can show a spinner. */
  opening: string | null;

  /** Load the category list once. No-op unless idle (retry via `retry`). */
  ensureCategories: () => void;
  /** Open a category and fetch its shelves. */
  select: (category: ExploreCategory) => void;
  /** Back to the category picker. */
  clear: () => void;
  /** Fetch an item's tracks and play them. */
  play: (item: ExploreItem) => Promise<void>;
  /** Re-run whichever load failed. */
  retry: () => void;
}

export const useExploreStore = create<ExploreStore>((set, get) => ({
  signedIn: false,
  sections: [],
  sectionsLoad: "idle",
  sectionsError: null,
  selected: null,
  shelves: [],
  pageLoad: "idle",
  pageError: null,
  opening: null,

  ensureCategories: () => {
    if (get().sectionsLoad !== "idle") return;
    set({ sectionsLoad: "loading", sectionsError: null });
    (async () => {
      // Ask about the account before browsing it: signed out isn't an error,
      // it's a different screen. (Mirrors `ensureYtMusic`.)
      const status = await ytmusicStatus();
      set({ signedIn: status.signedIn });
      if (!status.signedIn) {
        set({ sections: [], sectionsLoad: "ready" });
        return;
      }
      set({ sections: await ytmusicExploreCategories(), sectionsLoad: "ready" });
    })().catch((e) =>
      set({
        sections: [],
        sectionsLoad: "error",
        sectionsError: ipcErrorMessage(e),
      }),
    );
  },

  select: (category) => {
    const gen = ++pageGen;
    const isStale = () => gen !== pageGen;
    set({ selected: category, shelves: [], pageLoad: "loading", pageError: null });
    ytmusicExplorePage(category.params)
      .then((shelves) => {
        if (isStale()) return;
        set({ shelves, pageLoad: "ready" });
      })
      .catch((e) => {
        if (isStale()) return;
        set({ shelves: [], pageLoad: "error", pageError: ipcErrorMessage(e) });
      });
  },

  clear: () => {
    pageGen++; // abandon any in-flight page
    set({ selected: null, shelves: [], pageLoad: "idle", pageError: null });
  },

  play: async (item) => {
    // Opening is a network read; mark the tile so a slow album doesn't look dead.
    set({ opening: item.id });
    try {
      const tracks = await ytmusicExploreTracks(item);
      if (tracks.length === 0) {
        set({ pageError: `"${item.title}" has no playable tracks.` });
        return;
      }
      // Same queue items the library builds, so Explore rides the existing
      // streaming stack (Range seeking, resume-on-drop, gapless) unchanged.
      useEngineStore.getState().playQueueItems(tracks.map(ytmusicItem), 0);
    } catch (e) {
      set({ pageError: ipcErrorMessage(e) });
    } finally {
      set({ opening: null });
    }
  },

  retry: () => {
    const { selected, sectionsLoad } = get();
    if (sectionsLoad === "error") {
      set({ sectionsLoad: "idle" });
      get().ensureCategories();
      return;
    }
    if (selected) get().select(selected);
  },
}));
