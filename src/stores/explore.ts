import { create } from "zustand";
import {
  ipcErrorMessage,
  ytmusicExploreCategories,
  ytmusicExplorePage,
  ytmusicExploreTracks,
  ytmusicStatus,
} from "@/lib/ipc";
import type {
  ExploreCategory,
  ExploreItem,
  ExploreSection,
  ExploreShelf,
  YtTrack,
} from "@/lib/types";
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

  /** The playlist/album being viewed, with its tracks. Null = the shelves.
   *  Opening a tile shows what's in it rather than playing it outright — a
   *  hundred tracks starting unannounced is not a browse. */
  opened: { item: ExploreItem; tracks: YtTrack[] } | null;
  openError: string | null;

  /** Load the category list once. No-op unless idle (retry via `retry`). */
  ensureCategories: () => void;
  /** Open a category and fetch its shelves. */
  select: (category: ExploreCategory) => void;
  /** Back to the category picker. */
  clear: () => void;
  /** Fetch an item's tracks and show them. */
  open: (item: ExploreItem) => Promise<void>;
  /** Back to the shelves from an opened item. */
  close: () => void;
  /** Play the opened item's tracks from `index`. */
  playOpened: (index: number) => void;
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
  opened: null,
  openError: null,
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
    set({
      selected: category,
      shelves: [],
      pageLoad: "loading",
      pageError: null,
      opened: null,
      openError: null,
    });
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
    set({
      selected: null,
      shelves: [],
      pageLoad: "idle",
      pageError: null,
      opened: null,
      openError: null,
    });
  },

  open: async (item) => {
    // Opening is a network read; mark the tile so a slow album doesn't look dead.
    set({ opening: item.id, openError: null });
    try {
      const tracks = await ytmusicExploreTracks(item);
      set({
        opened: { item, tracks },
        openError: tracks.length === 0 ? `"${item.title}" has no playable tracks.` : null,
      });
    } catch (e) {
      set({ opened: { item, tracks: [] }, openError: ipcErrorMessage(e) });
    } finally {
      set({ opening: null });
    }
  },

  close: () => set({ opened: null, openError: null }),

  playOpened: (index) => {
    const opened = get().opened;
    if (!opened) return;
    // Unavailable tracks can't stream, so they never enter the queue — otherwise
    // next/prev would walk onto dead entries. The start index is re-based onto
    // the playable set, as the Library's `playAt` does.
    const target = opened.tracks[index];
    if (!target) return;
    const playable = opened.tracks.filter((t) => t.isAvailable);
    const from = playable.findIndex((t) => t.videoId === target.videoId);
    if (from < 0) return;
    // Same queue items the library builds, so Explore rides the existing
    // streaming stack (Range seeking, resume-on-drop, gapless) unchanged.
    useEngineStore.getState().playQueueItems(playable.map(ytmusicItem), from);
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
