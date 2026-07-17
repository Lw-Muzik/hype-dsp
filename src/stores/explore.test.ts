import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ExploreItem, ExploreShelf } from "@/lib/types";

// The store reaches for IPC as soon as it's imported, so it's mocked first.
vi.mock("@/lib/ipc", () => ({
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  ytmusicStatus: vi.fn(),
  ytmusicExploreCategories: vi.fn(),
  ytmusicExplorePage: vi.fn(),
  ytmusicExploreTracks: vi.fn(),
  ytmusicSearch: vi.fn(),
  ytmusicSearchSuggestions: vi.fn(),
  ytmusicArtistPage: vi.fn(),
}));

vi.mock("@/stores/engine", () => ({
  ytmusicItem: (t: unknown) => t,
  useEngineStore: { getState: () => ({ playQueueItems: playQueueItems }) },
}));

const playQueueItems = vi.fn();

const { useExploreStore } = await import("@/stores/explore");
const ipc = await import("@/lib/ipc");

const shelf = (title: string): ExploreShelf => ({ title, items: [] });

const item = (kind: ExploreItem["kind"], id: string): ExploreItem => ({
  kind,
  id,
  title: `${kind} ${id}`,
  subtitle: null,
  thumbnail: null,
});

/** A promise plus the handle to settle it, so a test can control arrival order. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.clearAllMocks();
  useExploreStore.setState({
    query: "",
    filter: "top",
    results: [],
    searchLoad: "idle",
    searchError: null,
    suggestions: [],
    selected: null,
    shelves: [],
    opened: null,
    artist: null,
    openError: null,
  });
});

describe("search", () => {
  it("keeps the query and the results it asked for", async () => {
    vi.mocked(ipc.ytmusicSearch).mockResolvedValue([shelf("Songs")]);
    useExploreStore.getState().search("burna boy");
    await vi.waitFor(() => expect(useExploreStore.getState().searchLoad).toBe("ready"));

    expect(ipc.ytmusicSearch).toHaveBeenCalledWith("burna boy", "top");
    expect(useExploreStore.getState().results).toEqual([shelf("Songs")]);
  });

  it("trims the query, and an empty one is not a search", () => {
    useExploreStore.getState().search("   ");
    expect(ipc.ytmusicSearch).not.toHaveBeenCalled();
    expect(useExploreStore.getState().query).toBe("");
  });

  it("puts a failure on screen rather than an empty result", async () => {
    vi.mocked(ipc.ytmusicSearch).mockRejectedValue(new Error("YouTube said no"));
    useExploreStore.getState().search("x");
    await vi.waitFor(() => expect(useExploreStore.getState().searchLoad).toBe("error"));
    expect(useExploreStore.getState().searchError).toBe("YouTube said no");
  });

  it("leaves nothing of a browse underneath it", async () => {
    useExploreStore.setState({
      selected: { title: "Afro", params: "p" },
      shelves: [shelf("Albums")],
    });
    vi.mocked(ipc.ytmusicSearch).mockResolvedValue([]);
    useExploreStore.getState().search("q");
    expect(useExploreStore.getState().selected).toBeNull();
    expect(useExploreStore.getState().shelves).toEqual([]);
  });

  /** Typing outruns the network. The first request can land last. */
  it("ignores an older search that arrives after a newer one", async () => {
    const first = deferred<ExploreShelf[]>();
    const second = deferred<ExploreShelf[]>();
    vi.mocked(ipc.ytmusicSearch)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    useExploreStore.getState().search("bur");
    useExploreStore.getState().search("burna boy");

    second.resolve([shelf("the answer they waited for")]);
    await vi.waitFor(() => expect(useExploreStore.getState().searchLoad).toBe("ready"));

    // The stale one lands late, and must not overwrite the newer answer.
    first.resolve([shelf("stale")]);
    await Promise.resolve();
    expect(useExploreStore.getState().results).toEqual([shelf("the answer they waited for")]);
    expect(useExploreStore.getState().query).toBe("burna boy");
  });

  it("does not let an abandoned search land after it was cleared", async () => {
    const inflight = deferred<ExploreShelf[]>();
    vi.mocked(ipc.ytmusicSearch).mockReturnValueOnce(inflight.promise);
    useExploreStore.getState().search("q");
    useExploreStore.getState().clearSearch();

    inflight.resolve([shelf("too late")]);
    await Promise.resolve();
    expect(useExploreStore.getState().results).toEqual([]);
    expect(useExploreStore.getState().searchLoad).toBe("idle");
  });

  it("re-asks under a new filter, keeping the query", async () => {
    vi.mocked(ipc.ytmusicSearch).mockResolvedValue([]);
    useExploreStore.getState().search("burna boy");
    await vi.waitFor(() => expect(useExploreStore.getState().searchLoad).toBe("ready"));

    useExploreStore.getState().setFilter("albums");
    expect(ipc.ytmusicSearch).toHaveBeenLastCalledWith("burna boy", "albums");
  });

  it("remembers a filter chosen before there's a query, without searching", () => {
    useExploreStore.getState().setFilter("artists");
    expect(ipc.ytmusicSearch).not.toHaveBeenCalled();
    expect(useExploreStore.getState().filter).toBe("artists");
  });
});

describe("suggestions", () => {
  it("are dropped once a real search has been made", async () => {
    const inflight = deferred<string[]>();
    vi.mocked(ipc.ytmusicSearchSuggestions).mockReturnValueOnce(inflight.promise);
    vi.mocked(ipc.ytmusicSearch).mockResolvedValue([]);

    useExploreStore.getState().suggest("burn");
    useExploreStore.getState().search("burna boy");
    inflight.resolve(["burna boy songs"]);
    await Promise.resolve();

    // Completions for a half-typed query are noise once the question was asked.
    expect(useExploreStore.getState().suggestions).toEqual([]);
  });

  it("never surface a failure — the search itself reports what matters", async () => {
    vi.mocked(ipc.ytmusicSearchSuggestions).mockRejectedValue(new Error("nope"));
    useExploreStore.getState().suggest("b");
    await vi.waitFor(() => expect(useExploreStore.getState().suggestions).toEqual([]));
  });
});

describe("open", () => {
  it("plays a song instead of listing one of it", async () => {
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([
      { videoId: "v1", isAvailable: true } as never,
    ]);
    await useExploreStore.getState().open(item("song", "v1"));

    expect(playQueueItems).toHaveBeenCalledWith([{ videoId: "v1", isAvailable: true }], 0);
    expect(useExploreStore.getState().opened).toBeNull();
  });

  it("says so rather than starting silence when a song can't be played", async () => {
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([
      { videoId: "v1", isAvailable: false } as never,
    ]);
    await useExploreStore.getState().open(item("song", "v1"));
    expect(playQueueItems).not.toHaveBeenCalled();
    expect(useExploreStore.getState().openError).toMatch(/can't be played/);
  });

  it("opens an artist as a page, not a track list", async () => {
    vi.mocked(ipc.ytmusicArtistPage).mockResolvedValue([shelf("Top songs")]);
    await useExploreStore.getState().open(item("artist", "UC1"));

    expect(useExploreStore.getState().artist?.shelves).toEqual([shelf("Top songs")]);
    expect(useExploreStore.getState().opened).toBeNull();
    expect(ipc.ytmusicExploreTracks).not.toHaveBeenCalled();
  });

  it("lists a playlist's tracks rather than playing them unasked", async () => {
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([
      { videoId: "a", isAvailable: true } as never,
      { videoId: "b", isAvailable: true } as never,
    ]);
    await useExploreStore.getState().open(item("playlist", "VL1"));

    expect(playQueueItems).not.toHaveBeenCalled();
    expect(useExploreStore.getState().opened?.tracks).toHaveLength(2);
  });

  /** Back should be one step, not a teleport out of wherever you are. */
  it("closes a track list back onto the artist page it was opened from", async () => {
    vi.mocked(ipc.ytmusicArtistPage).mockResolvedValue([shelf("Albums")]);
    await useExploreStore.getState().open(item("artist", "UC1"));
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([]);
    await useExploreStore.getState().open(item("album", "MPREb1"));

    useExploreStore.getState().close();
    expect(useExploreStore.getState().opened).toBeNull();
    expect(useExploreStore.getState().artist).not.toBeNull();

    useExploreStore.getState().close();
    expect(useExploreStore.getState().artist).toBeNull();
  });
});
