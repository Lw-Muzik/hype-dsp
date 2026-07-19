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

const { useExploreStore, searchQueue } = await import("@/stores/explore");
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

describe("searchQueue", () => {
  const results = (...items: ExploreItem[]): ExploreShelf[] => [
    { title: "Songs", items },
  ];

  it("queues every result, positioned on the one that was clicked", () => {
    const clicked = item("song", "v2");
    const q = searchQueue(results(item("song", "v1"), clicked, item("video", "v3")), clicked);

    expect(q?.tracks.map((t) => t.videoId)).toEqual(["v1", "v2", "v3"]);
    expect(q?.startIndex).toBe(1);
  });

  it("spans shelves in the order they're shown", () => {
    const clicked = item("video", "v9");
    const q = searchQueue(
      [
        { title: "Songs", items: [item("song", "v1")] },
        { title: "Videos", items: [clicked] },
      ],
      clicked,
    );
    expect(q?.tracks.map((t) => t.videoId)).toEqual(["v1", "v9"]);
    expect(q?.startIndex).toBe(1);
  });

  /** A card names a page to open. Queueing one would enqueue a track whose
   *  "video id" is a browse id, which resolves to nothing. */
  it("queues only what can be played, never a card", () => {
    const clicked = item("song", "v1");
    const q = searchQueue(
      results(clicked, item("album", "MPRE1"), item("artist", "UC1"), item("playlist", "VLPL1")),
      clicked,
    );
    expect(q?.tracks.map((t) => t.videoId)).toEqual(["v1"]);
  });

  /** The same track is routinely listed under both "Songs" and "Videos".
   *  Hearing it twice in a row is worse than not queueing it twice. */
  it("does not queue the same video twice", () => {
    const clicked = item("song", "v1");
    const q = searchQueue(
      [
        { title: "Songs", items: [clicked] },
        { title: "Videos", items: [item("video", "v1"), item("video", "v2")] },
      ],
      clicked,
    );
    expect(q?.tracks.map((t) => t.videoId)).toEqual(["v1", "v2"]);
    expect(q?.startIndex).toBe(0);
  });

  /** Opened from a shelf or an artist page, not from search — queueing the
   *  leftovers of an unrelated search would be worse than queueing nothing. */
  it("declines when the track isn't one of the results", () => {
    expect(searchQueue(results(item("song", "v1")), item("song", "other"))).toBeNull();
    expect(searchQueue([], item("song", "v1"))).toBeNull();
  });

  it("carries the fields the row already stated", () => {
    const clicked: ExploreItem = {
      ...item("video", "v1"),
      title: "Tombé",
      artist: "ELEMENT EleeeH",
      album: null,
      durationSecs: 224,
      thumbnail: "https://i.ytimg.com/x.jpg",
      hasVideo: true,
    };
    const track = searchQueue(results(clicked), clicked)!.tracks[0]!;

    expect(track).toMatchObject({
      videoId: "v1",
      title: "Tombé",
      artist: "ELEMENT EleeeH",
      durationSecs: 224,
      thumbnail: "https://i.ytimg.com/x.jpg",
      hasVideo: true,
      // Search lists nothing it won't serve; a blocked track fails at resolve.
      isAvailable: true,
    });
  });

  /** `hasVideo` decides whether the player offers a Video tab at all, so a row
   *  that didn't say must not be turned into a promise of footage. */
  it("does not promise video a row never claimed", () => {
    const clicked = item("song", "v1");
    expect(searchQueue(results(clicked), clicked)!.tracks[0]!.hasVideo).toBe(false);
  });
});

describe("playing a search result", () => {
  it("queues the rest of the results behind it", async () => {
    const clicked = item("song", "v2");
    useExploreStore.setState({
      results: [{ title: "Songs", items: [item("song", "v1"), clicked, item("video", "v3")] }],
    });
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([
      { videoId: "v2", isAvailable: true } as never,
    ]);

    await useExploreStore.getState().open(clicked);

    const [queue, from] = playQueueItems.mock.calls[0]!;
    expect(queue.map((t: { videoId: string }) => t.videoId)).toEqual(["v1", "v2", "v3"]);
    expect(from).toBe(1);
  });

  /** Nothing was searched, so there is no result set to continue into — the
   *  single-track behaviour is still the right one. */
  it("still plays a lone track when it came from a shelf", async () => {
    vi.mocked(ipc.ytmusicExploreTracks).mockResolvedValue([
      { videoId: "v1", isAvailable: true } as never,
    ]);
    await useExploreStore.getState().open(item("song", "v1"));

    expect(playQueueItems).toHaveBeenCalledWith([{ videoId: "v1", isAvailable: true }], 0);
  });
});
