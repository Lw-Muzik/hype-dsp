import { create } from "zustand";
import { ipcErrorMessage, moviesBrowse, moviesResolve, moviesSearch, ytMoviesChannel } from "@/lib/ipc";
import type { Movie, MovieDetail } from "@/lib/types";
import type { LoadStatus } from "@/stores/musicLibrary";

/**
 * Movies — modern (80s→now) films from licensed distributor YouTube channels
 * ("source: youtube"), grouped alongside public-domain / Creative-Commons
 * films from the Internet Archive ("source: archive", the "Classics" shelves).
 *
 * Shaped like `explore.ts`: nothing here is the user's own library, so nothing
 * is cached to disk — a shelf is fetched once per session and kept (neither
 * catalog churns minute to minute), a search is a fresh read every time, and
 * a stale response arriving late can never clobber a newer one
 * (`searchGen`/`detailGen`), the same guard Explore and the radio queue use.
 * Search stays Archive-only for v1: a YouTube channel's uploads have no
 * keyless search, only a listing (that needs the YouTube Data API).
 */

/** A confirmed Internet Archive collection that actually holds films
 *  (curl-verified `numFound > 0`). Mirrors `commands::movies::COLLECTIONS`
 *  in the Rust backend — keep the two in sync. Several plausible ids
 *  (`film_noir`, `comedy_films`, `documentaryfilms`) return zero and are
 *  deliberately left out of both lists. */
export interface MovieCollection {
  id: string;
  label: string;
}

export const MOVIE_COLLECTIONS: readonly MovieCollection[] = [
  { id: "feature_films", label: "Feature Films" },
  { id: "SciFi_Horror", label: "Sci-Fi & Horror" },
  { id: "silent_films", label: "Silent Films" },
  { id: "classic_cartoons", label: "Classic Cartoons" },
  { id: "classic_tv", label: "Classic TV" },
  { id: "prelinger", label: "Prelinger Archives" },
  { id: "short_films", label: "Short Films" },
];

/** A verified rights-holder YouTube distributor channel whose full-length
 *  uploads are browsable keylessly (yt-dlp `--flat-playlist` discovery —
 *  never a stream-extraction path; see `commands::movies_yt`).
 *
 *  **This is the single place to add another channel.** Append an entry only
 *  after confirming it live (curl/yt-dlp) actually publishes ≥60-minute
 *  uploads — WB's free-movie shelf was tried and skipped for v1 because it's
 *  geo-flaky, and a channel that merely *sounds* right isn't good enough. */
export interface YtMovieChannel {
  id: string;
  label: string;
}

export const YT_MOVIE_CHANNELS: readonly YtMovieChannel[] = [
  { id: "UC8IHAQMuiJdY6ALuhG7iU8Q", label: "FilmRise" },
  { id: "UCpHaAKu74UHvcYCi2g_PvBQ", label: "Shout! Studios" },
  { id: "UC2u3R3pjOiPZu4LtTlKkxdw", label: "Maverick Movies" },
  { id: "UCEK3tT7DcfWGWJpNEDBdWog", label: "Mosfilm" },
];

/** How many titles a YouTube shelf shows — enough to fill a shelf's scroll
 *  width without asking yt-dlp to enumerate a whole channel's history. */
const YT_SHELF_LIMIT = 24;

interface ShelfState {
  movies: Movie[];
  load: LoadStatus;
  error: string | null;
}

const idleShelf = (): ShelfState => ({ movies: [], load: "idle", error: null });

/** Only the newest search/detail request may ever write to state — an older
 *  one that resolves late (a slow shelf, a fast retype) is dropped. */
let searchGen = 0;
let detailGen = 0;

interface MoviesStore {
  /** Each collection's popular-films shelf, fetched lazily and kept once
   *  loaded — keyed by collection id. A shelf that failed reports its own
   *  error so one dead collection never blanks the others. */
  shelves: Record<string, ShelfState>;
  /** Fetch a shelf's first page if it hasn't been loaded (or tried) yet. */
  ensureShelf: (collectionId: string) => void;
  /** Re-run a shelf that failed. */
  retryShelf: (collectionId: string) => void;

  /** Same shape as `shelves`, one per YouTube distributor channel — kept as
   *  its own map (not merged into `shelves`) because Archive collection ids
   *  and YouTube channel ids are unrelated namespaces fetched through
   *  entirely different commands. */
  ytShelves: Record<string, ShelfState>;
  /** Fetch a channel's shelf if it hasn't been loaded (or tried) yet. */
  ensureYtShelf: (channelId: string) => void;
  /** Re-run a channel shelf that failed. */
  retryYtShelf: (channelId: string) => void;

  query: string;
  results: Movie[];
  searchLoad: LoadStatus;
  searchError: string | null;
  /** Search across the whitelisted collections. An empty/whitespace query
   *  clears back to the shelves rather than asking the backend. */
  search: (query: string) => void;
  clearSearch: () => void;

  /** The card that was clicked — its poster/title/year are already known, so
   *  the detail screen renders them instantly while the full detail
   *  (description + streams) resolves behind it. */
  opened: Movie | null;
  detail: MovieDetail | null;
  detailLoad: LoadStatus;
  detailError: string | null;
  open: (movie: Movie) => void;
  close: () => void;
  /** Re-run the resolve for whichever movie is open. */
  retryDetail: () => void;
}

export const useMoviesStore = create<MoviesStore>((set, get) => ({
  shelves: {},

  ensureShelf: (collectionId) => {
    const existing = get().shelves[collectionId];
    if (existing && existing.load !== "idle") return;
    set((s) => ({
      shelves: { ...s.shelves, [collectionId]: { movies: [], load: "loading", error: null } },
    }));
    moviesBrowse(collectionId, 1)
      .then((movies) => {
        set((s) => ({
          shelves: { ...s.shelves, [collectionId]: { movies, load: "ready", error: null } },
        }));
      })
      .catch((e) => {
        set((s) => ({
          shelves: {
            ...s.shelves,
            [collectionId]: { movies: [], load: "error", error: ipcErrorMessage(e) },
          },
        }));
      });
  },

  retryShelf: (collectionId) => {
    set((s) => ({ shelves: { ...s.shelves, [collectionId]: idleShelf() } }));
    get().ensureShelf(collectionId);
  },

  ytShelves: {},

  ensureYtShelf: (channelId) => {
    const existing = get().ytShelves[channelId];
    if (existing && existing.load !== "idle") return;
    set((s) => ({
      ytShelves: { ...s.ytShelves, [channelId]: { movies: [], load: "loading", error: null } },
    }));
    ytMoviesChannel(channelId, YT_SHELF_LIMIT)
      .then((movies) => {
        set((s) => ({
          ytShelves: { ...s.ytShelves, [channelId]: { movies, load: "ready", error: null } },
        }));
      })
      .catch((e) => {
        set((s) => ({
          ytShelves: {
            ...s.ytShelves,
            [channelId]: { movies: [], load: "error", error: ipcErrorMessage(e) },
          },
        }));
      });
  },

  retryYtShelf: (channelId) => {
    set((s) => ({ ytShelves: { ...s.ytShelves, [channelId]: idleShelf() } }));
    get().ensureYtShelf(channelId);
  },

  query: "",
  results: [],
  searchLoad: "idle",
  searchError: null,

  search: (query) => {
    const q = query.trim();
    if (!q) {
      get().clearSearch();
      return;
    }
    const gen = ++searchGen;
    const isStale = () => gen !== searchGen;
    set({ query: q, results: [], searchLoad: "loading", searchError: null });
    moviesSearch(q, 1)
      .then((results) => {
        if (isStale()) return;
        set({ results, searchLoad: "ready" });
      })
      .catch((e) => {
        if (isStale()) return;
        set({ results: [], searchLoad: "error", searchError: ipcErrorMessage(e) });
      });
  },

  clearSearch: () => {
    searchGen++; // abandon any in-flight search
    set({ query: "", results: [], searchLoad: "idle", searchError: null });
  },

  opened: null,
  detail: null,
  detailLoad: "idle",
  detailError: null,

  open: (movie) => {
    const gen = ++detailGen;
    const isStale = () => gen !== detailGen;
    if (movie.source === "youtube") {
      // Nothing to resolve: the channel listing already carried everything
      // this screen needs (title/thumbnail/duration/channel), and playback
      // only ever needs the video id — no per-item detail call exists.
      set({ opened: movie, detail: null, detailLoad: "ready", detailError: null });
      return;
    }
    set({ opened: movie, detail: null, detailLoad: "loading", detailError: null });
    moviesResolve(movie.identifier)
      .then((detail) => {
        if (isStale()) return;
        set({ detail, detailLoad: "ready" });
      })
      .catch((e) => {
        if (isStale()) return;
        set({ detailLoad: "error", detailError: ipcErrorMessage(e) });
      });
  },

  close: () => {
    detailGen++; // abandon any in-flight resolve
    set({ opened: null, detail: null, detailLoad: "idle", detailError: null });
  },

  retryDetail: () => {
    const opened = get().opened;
    // A YouTube movie has nothing async to retry — its "detail" was already
    // complete the instant it was opened.
    if (opened && opened.source === "archive") get().open(opened);
  },
}));
