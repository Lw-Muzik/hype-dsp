// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { Movie } from "@/lib/types";

// The store reaches for IPC as soon as an action runs, so it's mocked first.
vi.mock("@/lib/ipc", () => ({
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  moviesBrowse: vi.fn(),
  moviesSearch: vi.fn(),
  moviesResolve: vi.fn(),
}));

// `MovieDetailView` → `MoviePlayer` are statically imported by `MoviesView`
// even though `opened` stays null in every test below — these two modules
// are stubbed so that import graph resolves outside a real Tauri webview
// (no `window.__TAURI_INTERNALS__`, no live audio engine).
vi.mock("@/stores/engine", () => ({
  useEngineStore: Object.assign(
    (selector: (s: { stop: () => Promise<void> }) => unknown) => selector({ stop: vi.fn() }),
    { getState: () => ({ stop: vi.fn() }) },
  ),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setFullscreen: vi.fn().mockResolvedValue(undefined) }),
}));

// jsdom has no ResizeObserver; `VirtualGrid` (the search-results grid) needs one.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

const { MoviesView } = await import("@/features/movies/MoviesView");
const { useMoviesStore } = await import("@/stores/movies");
const ipc = await import("@/lib/ipc");

const movie = (identifier: string, over: Partial<Movie> = {}): Movie => ({
  identifier,
  title: `Movie ${identifier}`,
  year: 1950,
  posterUrl: `https://archive.org/services/img/${identifier}`,
  downloads: 10,
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

afterEach(() => {
  cleanup();
});

describe("<MoviesView> shelves", () => {
  it("renders every confirmed collection as a shelf, and its lazily-fetched movies", async () => {
    vi.mocked(ipc.moviesBrowse).mockImplementation((collection) =>
      Promise.resolve([movie(`${collection}_1`, { title: `${collection} pick` })]),
    );

    render(<MoviesView />);

    expect(screen.getByText("Feature Films")).toBeInTheDocument();
    expect(screen.getByText("Sci-Fi & Horror")).toBeInTheDocument();
    expect(screen.getByText("Silent Films")).toBeInTheDocument();
    expect(screen.getByText("Classic Cartoons")).toBeInTheDocument();
    expect(screen.getByText("Classic TV")).toBeInTheDocument();
    expect(screen.getByText("Prelinger Archives")).toBeInTheDocument();
    expect(screen.getByText("Short Films")).toBeInTheDocument();

    await waitFor(() => expect(screen.getByText("feature_films pick")).toBeInTheDocument());
    expect(ipc.moviesBrowse).toHaveBeenCalledWith("feature_films", 1);
    expect(ipc.moviesBrowse).toHaveBeenCalledWith("short_films", 1);
  });

  it("shows a shelf's own error without blanking the rest of the page", async () => {
    vi.mocked(ipc.moviesBrowse).mockImplementation((collection) =>
      collection === "feature_films" ? Promise.reject(new Error("Archive is down")) : Promise.resolve([]),
    );

    render(<MoviesView />);

    await waitFor(() => expect(screen.getByText("Archive is down")).toBeInTheDocument());
    // A dead shelf doesn't take the rest of the page down with it.
    expect(screen.getByText("Silent Films")).toBeInTheDocument();
  });
});

describe("<MoviesView> search", () => {
  it("renders a virtualized grid of results once a search resolves", async () => {
    vi.mocked(ipc.moviesBrowse).mockResolvedValue([]);
    vi.mocked(ipc.moviesSearch).mockResolvedValue([
      movie("nosferatu", { title: "Nosferatu" }),
      movie("metropolis", { title: "Metropolis" }),
    ]);

    render(<MoviesView />);

    // Drive the store directly rather than waiting out the input's debounce —
    // the debounce timing itself is the store's own `search()` contract,
    // already covered in movies.test.ts.
    useMoviesStore.getState().search("silent classics");

    await waitFor(() => expect(screen.getByText("Nosferatu")).toBeInTheDocument());
    expect(screen.getByText("Metropolis")).toBeInTheDocument();
    // A search replaces the browse screen entirely.
    expect(screen.queryByText("Feature Films")).not.toBeInTheDocument();
  });

  it("shows a no-results state rather than an empty grid", async () => {
    vi.mocked(ipc.moviesBrowse).mockResolvedValue([]);
    vi.mocked(ipc.moviesSearch).mockResolvedValue([]);

    render(<MoviesView />);
    useMoviesStore.getState().search("nothing matches this");

    await waitFor(() => expect(screen.getByText("No results")).toBeInTheDocument());
  });
});
