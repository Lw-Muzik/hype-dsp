import { describe, expect, it, vi } from "vitest";
import type { ExploreItem, ExploreShelf, LibraryTrack, YtTrack } from "@/lib/types";
import { executeRpc, MAX_LIBRARY_PAGE, type RpcPort } from "./rpc";
import { DEVICE, libTrack, ytTrack } from "./testKit";

const exploreItem = (id: string, over: Partial<ExploreItem> = {}): ExploreItem => ({
  kind: "song",
  id,
  title: `T ${id}`,
  subtitle: null,
  thumbnail: null,
  ...over,
});

function fakePort(over: Partial<RpcPort> & { library?: LibraryTrack[] } = {}) {
  const { library = [], ...rest } = over;
  const base = {
    deviceId: () => DEVICE,
    ytSignedIn: vi.fn(async () => true),
    search: vi.fn(async (_q: string, _f: string): Promise<ExploreShelf[]> => []),
    suggest: vi.fn(async () => [] as string[]),
    exploreTracks: vi.fn(async (_item: ExploreItem): Promise<YtTrack[]> => []),
    artistPage: vi.fn(async (): Promise<ExploreShelf[]> => []),
    localTracks: vi.fn(async () => library),
    forgetSignIn: vi.fn(),
    lyrics: { current: () => null, resolve: vi.fn(async () => null) },
  } satisfies RpcPort;
  // Overrides replace the mocks; the return type keeps `.mock` for the rest.
  return { ...base, ...rest } as typeof base;
}

describe("search", () => {
  it("maps the contract's `all` filter onto the app's `top`, others unchanged", async () => {
    const port = fakePort();
    await executeRpc("search", { query: "q", filter: "all" }, port);
    await executeRpc("search", { query: "q", filter: "songs" }, port);
    expect(port.search.mock.calls).toEqual([
      ["q", "top"],
      ["q", "songs"],
    ]);
  });

  it("converts durationSecs to durationMs and fills nullable fields", async () => {
    const port = fakePort({
      search: vi.fn(async () => [
        { title: "Songs", items: [exploreItem("s1", { durationSecs: 12.5, hasVideo: true })] },
      ]),
    });
    const out = await executeRpc("search", { query: "q", filter: "songs" }, port);
    expect(out).toEqual({
      ok: true,
      result: {
        shelves: [
          {
            title: "Songs",
            items: [
              {
                kind: "song",
                id: "s1",
                title: "T s1",
                subtitle: null,
                thumbnail: null,
                artist: null,
                album: null,
                durationMs: 12_500,
                hasVideo: true,
              },
            ],
          },
        ],
      },
    });
  });

  it("rejects a missing query or unknown filter without calling YouTube", async () => {
    const port = fakePort();
    expect(await executeRpc("search", { filter: "all" }, port)).toEqual({ ok: false, error: "failed" });
    expect(await executeRpc("search", { query: "q", filter: "podcasts" }, port)).toEqual({
      ok: false,
      error: "failed",
    });
    expect(port.search).not.toHaveBeenCalled();
  });
});

describe("signed-out handling", () => {
  it("YT rpcs answer not_signed_in from the cached status, without calling YT", async () => {
    const port = fakePort({ ytSignedIn: vi.fn(async () => false) });
    for (const m of ["search", "suggest", "open", "artist"] as const) {
      expect(await executeRpc(m, { query: "q", filter: "all", id: "x" }, port)).toEqual({
        ok: false,
        error: "not_signed_in",
      });
    }
    expect(port.search).not.toHaveBeenCalled();
    expect(port.suggest).not.toHaveBeenCalled();
  });

  it("library works while signed out of YouTube", async () => {
    const port = fakePort({ ytSignedIn: vi.fn(async () => false), library: [libTrack("/a")] });
    const out = await executeRpc("library", { offset: 0, limit: 10 }, port);
    expect(out.ok).toBe(true);
    expect(port.ytSignedIn).not.toHaveBeenCalled();
  });

  it("a 'Not signed in' failure from YT forgets the cache and reports not_signed_in", async () => {
    const port = fakePort({
      search: vi.fn(async () => {
        throw { message: "Not signed in to YouTube Music." };
      }),
    });
    expect(await executeRpc("search", { query: "q", filter: "all" }, port)).toEqual({
      ok: false,
      error: "not_signed_in",
    });
    expect(port.forgetSignIn).toHaveBeenCalledOnce();
  });

  it("any other failure is `failed` and keeps the cache", async () => {
    const port = fakePort({
      search: vi.fn(async () => {
        throw new Error("network down");
      }),
    });
    expect(await executeRpc("search", { query: "q", filter: "all" }, port)).toEqual({
      ok: false,
      error: "failed",
    });
    expect(port.forgetSignIn).not.toHaveBeenCalled();
  });
});

describe("open / artist / suggest", () => {
  const item = {
    kind: "album",
    id: "MPREb_1",
    title: "An Album",
    subtitle: null,
    thumbnail: null,
    artist: null,
    album: null,
    durationMs: 61_000,
    hasVideo: false,
  };

  it("open returns the title and only available tracks, as yt refs", async () => {
    const port = fakePort({
      exploreTracks: vi.fn(async () => [ytTrack("ok"), ytTrack("gone", { isAvailable: false })]),
    });
    const out = await executeRpc("open", { item }, port);
    expect(out).toMatchObject({ ok: true, result: { title: "An Album" } });
    const tracks = (out as unknown as { result: { tracks: { videoId: string; src: string }[] } }).result.tracks;
    expect(tracks.map((t) => [t.src, t.videoId])).toEqual([["yt", "ok"]]);
    // ms → secs on the way in
    expect(port.exploreTracks.mock.calls[0]![0].durationSecs).toBe(61);
  });

  it("open with a malformed item fails", async () => {
    const port = fakePort();
    expect(await executeRpc("open", { item: { kind: "nope" } }, port)).toEqual({ ok: false, error: "failed" });
    expect(port.exploreTracks).not.toHaveBeenCalled();
  });

  it("artist and suggest pass their argument through", async () => {
    const port = fakePort({ suggest: vi.fn(async () => ["a", "b"]) });
    expect(await executeRpc("suggest", { query: "ab" }, port)).toEqual({
      ok: true,
      result: { suggestions: ["a", "b"] },
    });
    await executeRpc("artist", { id: "UC123" }, port);
    expect(port.artistPage).toHaveBeenCalledWith("UC123");
    expect(await executeRpc("artist", {}, port)).toEqual({ ok: false, error: "failed" });
  });
});

describe("library", () => {
  const many = Array.from({ length: 450 }, (_, i) => libTrack(`/t${i}.mp3`, { title: `Track ${i}` }));
  const tracksOf = (o: unknown) =>
    (o as { result: { total: number; tracks: { id: string; deviceId: string; src: string }[] } }).result;

  it("pages with offset/limit and reports the unpaged total", async () => {
    const port = fakePort({ library: many });
    const r = tracksOf(await executeRpc("library", { offset: 10, limit: 5 }, port));
    expect(r.total).toBe(450);
    expect(r.tracks.map((t) => t.id)).toEqual(["/t10.mp3", "/t11.mp3", "/t12.mp3", "/t13.mp3", "/t14.mp3"]);
    expect(r.tracks.every((t) => t.src === "local" && t.deviceId === DEVICE)).toBe(true);
  });

  it("caps a page at 200 however much is asked for", async () => {
    const port = fakePort({ library: many });
    const r = tracksOf(await executeRpc("library", { offset: 0, limit: 5000 }, port));
    expect(r.tracks).toHaveLength(MAX_LIBRARY_PAGE);
    expect(MAX_LIBRARY_PAGE).toBe(200);
  });

  it("filters case-insensitively over title, artist and album, and totals the matches", async () => {
    const lib = [
      libTrack("/1", { title: "Sura Yako", artist: "Ay", album: null }),
      libTrack("/2", { title: "Other", artist: "SURA Band", album: null }),
      libTrack("/3", { title: "Other", artist: null, album: "The Sura Years" }),
      libTrack("/4", { title: "Nope", artist: "Nobody", album: "None" }),
    ];
    const port = fakePort({ library: lib });
    const r = tracksOf(await executeRpc("library", { offset: 0, limit: 50, query: "  sURa " }, port));
    expect(r.total).toBe(3);
    expect(r.tracks.map((t) => t.id)).toEqual(["/1", "/2", "/3"]);
  });

  it("requires numeric offset and limit", async () => {
    const port = fakePort({ library: many });
    expect(await executeRpc("library", { limit: 5 }, port)).toEqual({ ok: false, error: "failed" });
    expect(await executeRpc("library", { offset: 0, limit: "5" }, port)).toEqual({ ok: false, error: "failed" });
  });

  it("an offset past the end is an empty page, not an error", async () => {
    const port = fakePort({ library: many });
    const r = tracksOf(await executeRpc("library", { offset: 9999, limit: 10 }, port));
    expect(r.tracks).toEqual([]);
    expect(r.total).toBe(450);
  });
});
