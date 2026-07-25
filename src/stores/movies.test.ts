import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Movie, MovieDetail } from "@/lib/types";

// The store reaches for IPC as soon as an action runs, so it's mocked first —
// same setup as explore.test.ts.
vi.mock("@/lib/ipc", () => ({
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  moviesBrowse: vi.fn(),
  moviesSearch: vi.fn(),
  moviesResolve: vi.fn(),
}));

const { useMoviesStore, MOVIE_COLLECTIONS } = await import("@/stores/movies");
const ipc = await import("@/lib/ipc");

/** A promise plus the handle to settle it, so a test can control arrival order. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const movie = (identifier: string, over: Partial<Movie> = {}): Movie => ({
  identifier,
  title: `Movie ${identifier}`,
  year: 1955,
  posterUrl: `https://archive.org/services/img/${identifier}`,
  downloads: 1000,
  ...over,
});

const detail = (identifier: string, over: Partial<MovieDetail> = {}): MovieDetail => ({
  identifier,
  title: `Movie ${identifier}`,
  year: 1955,
  description: "A film.",
  posterUrl: `https://archive.org/services/img/${identifier}`,
  streams: [{ label: "SD", url: `https://archive.org/download/${identifier}/sd.mp4`, mime: "video/mp4" }],
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  useMoviesStore.setState({
    shelves: {},
    query: "",
    results: [],
    searchLoad: "idle",
    searchError: null,
    opened: null,
    detail: null,
    detailLoad: "idle",
    detailError: null,
  });
});

describe("MOVIE_COLLECTIONS", () => {
  it("lists exactly the confirmed collections from the spec", () => {
    expect(MOVIE_COLLECTIONS.map((c) => c.id)).toEqual([
      "feature_films",
      "SciFi_Horror",
      "silent_films",
      "classic_cartoons",
      "classic_tv",
      "prelinger",
      "short_films",
    ]);
  });
});

describe("ensureShelf", () => {
  it("loads a collection's shelf once", async () => {
    vi.mocked(ipc.moviesBrowse).mockResolvedValue([movie("a"), movie("b")]);
    useMoviesStore.getState().ensureShelf("feature_films");
    expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("loading");

    await vi.waitFor(() =>
      expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("ready"),
    );
    expect(useMoviesStore.getState().shelves.feature_films?.movies).toHaveLength(2);
    expect(ipc.moviesBrowse).toHaveBeenCalledWith("feature_films", 1);
  });

  it("does not refetch a shelf that's already loading or loaded", async () => {
    vi.mocked(ipc.moviesBrowse).mockResolvedValue([movie("a")]);
    useMoviesStore.getState().ensureShelf("feature_films");
    useMoviesStore.getState().ensureShelf("feature_films"); // second call, still in flight
    await vi.waitFor(() =>
      expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("ready"),
    );
    useMoviesStore.getState().ensureShelf("feature_films"); // third call, already ready
    expect(ipc.moviesBrowse).toHaveBeenCalledTimes(1);
  });

  it("keeps one collection's failure from touching another's shelf", async () => {
    vi.mocked(ipc.moviesBrowse)
      .mockRejectedValueOnce(new Error("Archive is down"))
      .mockResolvedValueOnce([movie("a")]);
    useMoviesStore.getState().ensureShelf("feature_films");
    useMoviesStore.getState().ensureShelf("silent_films");

    await vi.waitFor(() => {
      expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("error");
      expect(useMoviesStore.getState().shelves.silent_films?.load).toBe("ready");
    });
    expect(useMoviesStore.getState().shelves.feature_films?.error).toBe("Archive is down");
    expect(useMoviesStore.getState().shelves.silent_films?.movies).toHaveLength(1);
  });

  it("retryShelf resets to idle and fetches again", async () => {
    vi.mocked(ipc.moviesBrowse).mockRejectedValueOnce(new Error("nope"));
    useMoviesStore.getState().ensureShelf("feature_films");
    await vi.waitFor(() =>
      expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("error"),
    );

    vi.mocked(ipc.moviesBrowse).mockResolvedValueOnce([movie("a")]);
    useMoviesStore.getState().retryShelf("feature_films");
    await vi.waitFor(() =>
      expect(useMoviesStore.getState().shelves.feature_films?.load).toBe("ready"),
    );
    expect(ipc.moviesBrowse).toHaveBeenCalledTimes(2);
  });
});

describe("search", () => {
  it("keeps the query and the results it asked for", async () => {
    vi.mocked(ipc.moviesSearch).mockResolvedValue([movie("night_of_the_living_dead")]);
    useMoviesStore.getState().search("night of the living dead");
    await vi.waitFor(() => expect(useMoviesStore.getState().searchLoad).toBe("ready"));

    expect(ipc.moviesSearch).toHaveBeenCalledWith("night of the living dead", 1);
    expect(useMoviesStore.getState().results).toEqual([movie("night_of_the_living_dead")]);
  });

  it("trims the query, and an empty one is not a search", () => {
    useMoviesStore.getState().search("   ");
    expect(ipc.moviesSearch).not.toHaveBeenCalled();
    expect(useMoviesStore.getState().query).toBe("");
  });

  it("puts a failure on screen rather than an empty result", async () => {
    vi.mocked(ipc.moviesSearch).mockRejectedValue(new Error("Archive said no"));
    useMoviesStore.getState().search("x");
    await vi.waitFor(() => expect(useMoviesStore.getState().searchLoad).toBe("error"));
    expect(useMoviesStore.getState().searchError).toBe("Archive said no");
  });

  /** Typing outruns the network — the first request can land last. */
  it("ignores an older search that arrives after a newer one", async () => {
    const first = deferred<Movie[]>();
    const second = deferred<Movie[]>();
    vi.mocked(ipc.moviesSearch).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    useMoviesStore.getState().search("nos");
    useMoviesStore.getState().search("nosferatu");

    second.resolve([movie("nosferatu")]);
    await vi.waitFor(() => expect(useMoviesStore.getState().searchLoad).toBe("ready"));

    first.resolve([movie("stale")]);
    await Promise.resolve();
    expect(useMoviesStore.getState().results).toEqual([movie("nosferatu")]);
    expect(useMoviesStore.getState().query).toBe("nosferatu");
  });

  it("does not let an abandoned search land after it was cleared", async () => {
    const inflight = deferred<Movie[]>();
    vi.mocked(ipc.moviesSearch).mockReturnValueOnce(inflight.promise);
    useMoviesStore.getState().search("q");
    useMoviesStore.getState().clearSearch();

    inflight.resolve([movie("too_late")]);
    await Promise.resolve();
    expect(useMoviesStore.getState().results).toEqual([]);
    expect(useMoviesStore.getState().searchLoad).toBe("idle");
  });
});

describe("open / detail resolution", () => {
  it("shows the clicked card at once, then fills in the resolved detail", async () => {
    const card = movie("metropolis");
    const resolved = detail("metropolis", { description: "A dystopian city." });
    vi.mocked(ipc.moviesResolve).mockResolvedValue(resolved);

    useMoviesStore.getState().open(card);
    expect(useMoviesStore.getState().opened).toEqual(card);
    expect(useMoviesStore.getState().detailLoad).toBe("loading");

    await vi.waitFor(() => expect(useMoviesStore.getState().detailLoad).toBe("ready"));
    expect(useMoviesStore.getState().detail).toEqual(resolved);
  });

  it("reports a resolve failure without discarding the card already shown", async () => {
    vi.mocked(ipc.moviesResolve).mockRejectedValue(new Error("gone"));
    useMoviesStore.getState().open(movie("dead_link"));
    await vi.waitFor(() => expect(useMoviesStore.getState().detailLoad).toBe("error"));
    expect(useMoviesStore.getState().detailError).toBe("gone");
    expect(useMoviesStore.getState().opened?.identifier).toBe("dead_link");
  });

  it("ignores a slower resolve for a movie the user already navigated away from", async () => {
    const first = deferred<MovieDetail>();
    const second = deferred<MovieDetail>();
    vi.mocked(ipc.moviesResolve).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    useMoviesStore.getState().open(movie("a"));
    useMoviesStore.getState().open(movie("b"));

    second.resolve(detail("b"));
    await vi.waitFor(() => expect(useMoviesStore.getState().detailLoad).toBe("ready"));

    first.resolve(detail("a"));
    await Promise.resolve();
    expect(useMoviesStore.getState().detail?.identifier).toBe("b");
  });

  it("close abandons an in-flight resolve and clears the screen", async () => {
    const inflight = deferred<MovieDetail>();
    vi.mocked(ipc.moviesResolve).mockReturnValueOnce(inflight.promise);
    useMoviesStore.getState().open(movie("a"));
    useMoviesStore.getState().close();

    inflight.resolve(detail("a"));
    await Promise.resolve();
    expect(useMoviesStore.getState().opened).toBeNull();
    expect(useMoviesStore.getState().detail).toBeNull();
    expect(useMoviesStore.getState().detailLoad).toBe("idle");
  });

  it("retryDetail re-resolves the currently opened movie", async () => {
    vi.mocked(ipc.moviesResolve).mockRejectedValueOnce(new Error("nope"));
    useMoviesStore.getState().open(movie("a"));
    await vi.waitFor(() => expect(useMoviesStore.getState().detailLoad).toBe("error"));

    vi.mocked(ipc.moviesResolve).mockResolvedValueOnce(detail("a"));
    useMoviesStore.getState().retryDetail();
    await vi.waitFor(() => expect(useMoviesStore.getState().detailLoad).toBe("ready"));
    expect(ipc.moviesResolve).toHaveBeenCalledTimes(2);
  });
});
