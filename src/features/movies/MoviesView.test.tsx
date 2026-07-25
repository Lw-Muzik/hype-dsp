// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { ArchiveMovie, YoutubeMovie } from "@/lib/types";

// The store reaches for IPC as soon as an action runs, so it's mocked first.
vi.mock("@/lib/ipc", () => ({
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  moviesBrowse: vi.fn(),
  moviesSearch: vi.fn(),
  moviesResolve: vi.fn(),
  ytMoviesChannel: vi.fn(),
  openExternalUrl: vi.fn(),
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

const movie = (identifier: string, over: Partial<ArchiveMovie> = {}): ArchiveMovie => ({
  source: "archive",
  identifier,
  title: `Movie ${identifier}`,
  year: 1950,
  posterUrl: `https://archive.org/services/img/${identifier}`,
  downloads: 10,
  ...over,
});

const ytMovie = (videoId: string, over: Partial<YoutubeMovie> = {}): YoutubeMovie => ({
  source: "youtube",
  videoId,
  title: `YT Movie ${videoId}`,
  durationSec: 5400,
  posterUrl: `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`,
  channelTitle: "FilmRise",
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  // Every render of <MoviesView> mounts the four YouTube shelves alongside
  // the Archive ones — default them to an empty, resolved listing so tests
  // that don't care about the YouTube path aren't left with a hanging
  // promise or an unrelated error state.
  vi.mocked(ipc.ytMoviesChannel).mockResolvedValue([]);
  useMoviesStore.setState({
    shelves: {},
    ytShelves: {},
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

    // The Archive collections are grouped under one "Classics" heading.
    expect(screen.getByText("Classics")).toBeInTheDocument();
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

describe("<MoviesView> YouTube distributor shelves", () => {
  it("renders every verified channel as a shelf, and its lazily-fetched movies", async () => {
    vi.mocked(ipc.ytMoviesChannel).mockImplementation((channelId) =>
      Promise.resolve([ytMovie(`${channelId}_1`, { title: `${channelId} pick` })]),
    );

    render(<MoviesView />);

    expect(screen.getByText("FilmRise")).toBeInTheDocument();
    expect(screen.getByText("Shout! Studios")).toBeInTheDocument();
    expect(screen.getByText("Maverick Movies")).toBeInTheDocument();
    expect(screen.getByText("Mosfilm")).toBeInTheDocument();

    await waitFor(() =>
      expect(screen.getByText("UC8IHAQMuiJdY6ALuhG7iU8Q pick")).toBeInTheDocument(),
    );
    expect(ipc.ytMoviesChannel).toHaveBeenCalledWith("UC8IHAQMuiJdY6ALuhG7iU8Q", expect.any(Number));
    expect(ipc.ytMoviesChannel).toHaveBeenCalledWith("UCEK3tT7DcfWGWJpNEDBdWog", expect.any(Number));
  });

  it("shows a channel shelf's own error (e.g. yt-dlp missing) without blanking the rest of the page", async () => {
    vi.mocked(ipc.ytMoviesChannel).mockImplementation((channelId) =>
      channelId === "UC8IHAQMuiJdY6ALuhG7iU8Q"
        ? Promise.reject(new Error("yt-dlp isn't installed"))
        : Promise.resolve([]),
    );

    render(<MoviesView />);

    await waitFor(() => expect(screen.getByText("yt-dlp isn't installed")).toBeInTheDocument());
    // A dead channel shelf doesn't take the rest of the page down with it.
    expect(screen.getByText("Mosfilm")).toBeInTheDocument();
  });
});

describe("<MoviesView> detail routing by source", () => {
  it("opening a YouTube card shows the YouTube detail screen with no SD/HD toggle", async () => {
    vi.mocked(ipc.ytMoviesChannel).mockImplementation((channelId) =>
      channelId === "UC8IHAQMuiJdY6ALuhG7iU8Q"
        ? Promise.resolve([ytMovie("dQw4w9WgXcQ", { title: "A Licensed Full Movie" })])
        : Promise.resolve([]),
    );

    render(<MoviesView />);

    const card = await screen.findByText("A Licensed Full Movie");
    fireEvent.click(card);

    // The YouTube branch has a single Play button and no quality toggle —
    // `movies_resolve` is never called for it.
    await waitFor(() => expect(screen.getByRole("button", { name: /play/i })).toBeInTheDocument());
    expect(screen.queryByRole("radiogroup", { name: /video quality/i })).not.toBeInTheDocument();
    expect(ipc.moviesResolve).not.toHaveBeenCalled();
  });

  it("opening an Archive card resolves detail and offers the classics screen", async () => {
    vi.mocked(ipc.moviesBrowse).mockImplementation((collection) =>
      collection === "feature_films" ? Promise.resolve([movie("metropolis")]) : Promise.resolve([]),
    );
    vi.mocked(ipc.moviesResolve).mockResolvedValue({
      identifier: "metropolis",
      title: "Metropolis",
      year: 1927,
      description: "A dystopian city.",
      posterUrl: "https://archive.org/services/img/metropolis",
      streams: [{ label: "SD", url: "https://archive.org/download/metropolis/sd.mp4", mime: "video/mp4" }],
    });

    render(<MoviesView />);

    const card = await screen.findByText("Movie metropolis");
    fireEvent.click(card);

    await waitFor(() => expect(screen.getByText("A dystopian city.")).toBeInTheDocument());
    expect(ipc.moviesResolve).toHaveBeenCalledWith("metropolis");
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
