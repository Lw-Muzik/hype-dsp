import { create } from "zustand";
import {
  ipcErrorMessage,
  ytmusicArtistPage,
  ytmusicExploreCategories,
  ytmusicExplorePage,
  ytmusicExploreTracks,
  ytmusicSearch,
  ytmusicSearchSuggestions,
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
/** Likewise for searches — typing outruns the network, and an older answer
 *  arriving late must not overwrite a newer one. */
let searchGen = 0;

/** The filters YouTube offers, in the order it offers them. */
export const SEARCH_FILTERS = ["top", "songs", "videos", "albums", "artists", "playlists"] as const;
export type SearchFilter = (typeof SEARCH_FILTERS)[number];

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

  /** The artist being viewed, and their shelves. An artist isn't a track list —
   *  it's a page of them — so it can't ride `opened`. */
  artist: { item: ExploreItem; shelves: ExploreShelf[] } | null;

  /** The query as typed. Empty = not searching, which is what decides the
   *  screen: a search is a different view of the catalog, not a filter of the
   *  one you're on. */
  query: string;
  filter: SearchFilter;
  results: ExploreShelf[];
  searchLoad: LoadStatus;
  searchError: string | null;
  suggestions: string[];

  /** Load the category list once. No-op unless idle (retry via `retry`). */
  ensureCategories: () => void;
  /** Open a category and fetch its shelves. */
  select: (category: ExploreCategory) => void;
  /** Back to the category picker. */
  clear: () => void;
  /** Open an item: a song plays, an artist opens their page, anything else
   *  lists its tracks. */
  open: (item: ExploreItem) => Promise<void>;
  /** Back to the shelves from an opened item. */
  close: () => void;
  /** Play the opened item's tracks from `index`. */
  playOpened: (index: number) => void;
  /** Run a search. Empty query clears back to browsing. */
  search: (query: string, filter?: SearchFilter) => void;
  /** Re-run the current query under a different filter. */
  setFilter: (filter: SearchFilter) => void;
  /** Ask YouTube to complete a partial query. */
  suggest: (query: string) => void;
  /** Abandon the search and go back to browsing. */
  clearSearch: () => void;
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
  artist: null,
  pageLoad: "idle",
  pageError: null,
  opening: null,
  query: "",
  filter: "top",
  results: [],
  searchLoad: "idle",
  searchError: null,
  suggestions: [],

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
      artist: null,
      openError: null,
    });
  },

  open: async (item) => {
    // Opening is a network read; mark the tile so a slow album doesn't look dead.
    set({ opening: item.id, openError: null });
    try {
      if (item.kind === "artist") {
        set({ artist: { item, shelves: await ytmusicArtistPage(item.id) } });
        return;
      }
      const tracks = await ytmusicExploreTracks(item);
      // A song is not a list of one. Listing it would make the user click twice
      // to do the only thing a song row is for — the "don't play a hundred
      // tracks unasked" rule is about a hundred tracks, not about this.
      if (item.kind === "song" || item.kind === "video") {
        const playable = tracks.filter((t) => t.isAvailable);
        if (playable.length === 0) {
          set({ openError: `"${item.title}" can't be played.` });
          return;
        }
        // One click = this song plus its radio: the queue behind it fills with
        // YT Music's own similar-track picks (engine.playYtRadio), exactly as
        // the YT Music client does. This replaces the old "queue the rest of
        // the search page" behaviour: search results are what matched the
        // words; the radio is what matches the taste.
        useEngineStore.getState().playYtRadio(playable[0]!);
        return;
      }
      set({
        opened: { item, tracks },
        openError: tracks.length === 0 ? `"${item.title}" has no playable tracks.` : null,
      });
    } catch (e) {
      if (item.kind === "song" || item.kind === "video" || item.kind === "artist") {
        set({ openError: ipcErrorMessage(e) });
      } else {
        set({ opened: { item, tracks: [] }, openError: ipcErrorMessage(e) });
      }
    } finally {
      set({ opening: null });
    }
  },

  close: () => {
    // One step, innermost first: a track list opened from an artist's page goes
    // back to that page, not past it to wherever the artist was found.
    if (get().opened) {
      set({ opened: null, openError: null });
      return;
    }
    set({ artist: null, openError: null });
  },

  search: (query, filter) => {
    const q = query.trim();
    const next = filter ?? get().filter;
    if (!q) {
      get().clearSearch();
      return;
    }
    const gen = ++searchGen;
    const isStale = () => gen !== searchGen;
    // A search replaces whatever was being browsed: leaving a category open
    // underneath would make Back ambiguous about what it's going back to.
    set({
      query: q,
      filter: next,
      results: [],
      searchLoad: "loading",
      searchError: null,
      selected: null,
      shelves: [],
      opened: null,
      artist: null,
      openError: null,
      suggestions: [],
    });
    ytmusicSearch(q, next)
      .then((results) => {
        if (isStale()) return;
        set({ results, searchLoad: "ready" });
      })
      .catch((e) => {
        if (isStale()) return;
        set({ results: [], searchLoad: "error", searchError: ipcErrorMessage(e) });
      });
  },

  setFilter: (filter) => {
    const { query } = get();
    if (!query) {
      set({ filter });
      return;
    }
    get().search(query, filter);
  },

  suggest: (query) => {
    const q = query.trim();
    if (!q) {
      set({ suggestions: [] });
      return;
    }
    const gen = searchGen;
    ytmusicSearchSuggestions(q)
      .then((suggestions) => {
        // Anything that happened since — a search, a keystroke — outranks this.
        if (gen !== searchGen || get().query) return;
        set({ suggestions });
      })
      .catch(() => set({ suggestions: [] }));
  },

  clearSearch: () => {
    searchGen++; // abandon any in-flight search
    set({
      query: "",
      results: [],
      searchLoad: "idle",
      searchError: null,
      suggestions: [],
      opened: null,
      artist: null,
      openError: null,
    });
  },

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
    const { selected, sectionsLoad, query, searchLoad } = get();
    if (searchLoad === "error" && query) {
      get().search(query);
      return;
    }
    if (sectionsLoad === "error") {
      set({ sectionsLoad: "idle" });
      get().ensureCategories();
      return;
    }
    if (selected) get().select(selected);
  },
}));
