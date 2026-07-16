import { beforeEach, describe, expect, it, vi } from "vitest";
import type { YtMusicPage, YtMusicStatus, YtTrack } from "@/lib/types";

// The store calls the IPC layer at module scope on load, so it's mocked before
// the store is imported. Only the YT Music commands matter here; the rest resolve
// empty so the other sources' `ensure*` can't interfere.
vi.mock("@/lib/ipc", () => ({
  // Mirrors the real helper's Error branch; the store only ever hands it a
  // rejection value.
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  ytmusicStatus: vi.fn(),
  ytmusicAllTracks: vi.fn(),
  cloudAllAudio: vi.fn(),
  cloudCachedTags: vi.fn(),
  cloudStatus: vi.fn(),
  cloudTrackTags: vi.fn(),
  libraryAvailableCount: vi.fn(),
  libraryCount: vi.fn(),
  libraryListPage: vi.fn(),
  linkLibrary: vi.fn(),
  linkPaired: vi.fn(),
}));

const { useMusicLibraryStore, ytTrackToTrack } = await import("@/stores/musicLibrary");
const ipc = await import("@/lib/ipc");

const ytmusicStatus = vi.mocked(ipc.ytmusicStatus);
const ytmusicAllTracks = vi.mocked(ipc.ytmusicAllTracks);

const track = (over: Partial<YtTrack> = {}): YtTrack => ({
  videoId: "vid1",
  title: "Song",
  artist: "Artist",
  album: "Album",
  durationSecs: 210,
  thumbnail: "https://i.ytimg.com/vi/vid1/hq.jpg",
  playlistId: "pl1",
  playlistTitle: "Liked Music",
  isAvailable: true,
  ...over,
});

const status = (signedIn: boolean, present = true): YtMusicStatus => ({
  signedIn,
  ytdlp: { present, version: present ? "2026.01.01" : null, path: null, haveFfmpeg: present },
});

const page = (tracks: YtTrack[], fromCache: boolean): YtMusicPage => ({
  playlists: [
    { id: "pl1", title: "Liked Music", author: "me", trackCount: tracks.length, thumbnail: null },
  ],
  tracks,
  fromCache,
});

/** Reset to a pristine, signed-out, never-loaded store. Invalidating first is
 *  what makes each test hermetic: it bumps the generation token, so a previous
 *  test's still-in-flight load can't resolve into this one's state. */
function resetStore() {
  useMusicLibraryStore.getState().invalidateYtMusic();
  useMusicLibraryStore.setState({
    ytmusic: [],
    ytPlaylists: [],
    ytdlp: null,
    ytmusicLoad: "idle",
    ytmusicSignedIn: false,
  });
}

const settled = () => useMusicLibraryStore.getState().ytmusicLoad === "ready";

describe("ytTrackToTrack", () => {
  it("maps a track onto a browsable MusicTrack", () => {
    const t = ytTrackToTrack(track());
    expect(t).toMatchObject({
      uid: "ytmusic:vid1",
      source: "ytmusic",
      id: "vid1",
      title: "Song",
      artist: "Artist",
      album: "Album",
      durationSecs: 210,
      genre: null,
      artPath: null,
    });
  });

  it("groups by playlist under the Folders facet", () => {
    // The whole browse design: playlists ride the existing facet for free.
    expect(ytTrackToTrack(track({ playlistTitle: "Workout" })).folder).toBe("Workout");
  });

  it("uses the thumbnail as the cover", () => {
    expect(ytTrackToTrack(track()).cover).toBe("https://i.ytimg.com/vi/vid1/hq.jpg");
    expect(ytTrackToTrack(track({ thumbnail: null })).cover).toBeNull();
  });

  it("keys the uid by videoId so two playlists' copies collapse predictably", () => {
    // The same video in two playlists yields the same uid — deliberate: it's
    // one track, and React keys must not collide with a *different* video.
    expect(ytTrackToTrack(track({ playlistId: "a" })).uid).toBe(
      ytTrackToTrack(track({ playlistId: "b" })).uid,
    );
    expect(ytTrackToTrack(track({ videoId: "other" })).uid).toBe("ytmusic:other");
  });

  it("keeps isAvailable reachable so the UI can mark it unplayable", () => {
    expect(ytTrackToTrack(track({ isAvailable: false })).ytTrack?.isAvailable).toBe(false);
  });
});

describe("ensureYtMusic", () => {
  beforeEach(() => {
    // reset, not clear: `clearAllMocks` leaves queued `mockResolvedValueOnce`
    // values in place, so a test that doesn't consume its queue (e.g. one where
    // the refresh is skipped) would leak it into the next test's first call.
    vi.resetAllMocks();
    resetStore();
  });

  it("settles signed-out without ever listing", async () => {
    ytmusicStatus.mockResolvedValue(status(false));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() => expect(settled()).toBe(true));

    // Listing while signed out is a guaranteed error — don't make the call.
    expect(ytmusicAllTracks).not.toHaveBeenCalled();
    expect(useMusicLibraryStore.getState().ytmusicSignedIn).toBe(false);
    expect(useMusicLibraryStore.getState().ytmusic).toEqual([]);
  });

  it("records yt-dlp's state even when signed out", async () => {
    ytmusicStatus.mockResolvedValue(status(false, false));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() => expect(settled()).toBe(true));

    expect(useMusicLibraryStore.getState().ytdlp?.present).toBe(false);
  });

  it("serves the cache first, then refreshes behind it", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks
      .mockResolvedValueOnce(page([track({ videoId: "cached" })], true))
      .mockResolvedValueOnce(
        page([track({ videoId: "cached" }), track({ videoId: "fresh" })], false),
      );

    useMusicLibraryStore.getState().ensureYtMusic();

    // Phase 1 publishes the cached listing and is already usable...
    await vi.waitFor(() => expect(settled()).toBe(true));
    expect(ytmusicAllTracks).toHaveBeenNthCalledWith(1, false);

    // ...then phase 2 re-lists and republishes.
    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusic).toHaveLength(2),
    );
    expect(ytmusicAllTracks).toHaveBeenNthCalledWith(2, true);
    expect(useMusicLibraryStore.getState().ytmusic.map((t) => t.id)).toEqual([
      "cached",
      "fresh",
    ]);
  });

  it("skips the refresh when the first listing was already live", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks.mockResolvedValue(page([track()], false));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() => expect(settled()).toBe(true));
    // Give a phase 2 that shouldn't exist a chance to fire.
    await new Promise((r) => setTimeout(r, 10));

    expect(ytmusicAllTracks).toHaveBeenCalledTimes(1);
    expect(ytmusicAllTracks).toHaveBeenCalledWith(false);
  });

  it("keeps the cached tracks when the background refresh fails", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks
      .mockResolvedValueOnce(page([track({ videoId: "cached" })], true))
      .mockRejectedValueOnce(new Error("network went away"));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() => expect(settled()).toBe(true));
    await new Promise((r) => setTimeout(r, 10));

    // A failed *refresh* must not empty a source that already showed tracks.
    expect(useMusicLibraryStore.getState().ytmusic.map((t) => t.id)).toEqual(["cached"]);
    expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("ready");
  });

  it("discards a load invalidated mid-flight", async () => {
    let release!: (s: YtMusicStatus) => void;
    ytmusicStatus.mockReturnValue(
      new Promise<YtMusicStatus>((r) => {
        release = r;
      }),
    );

    useMusicLibraryStore.getState().ensureYtMusic();
    // Sign-out lands while the status call is still in flight.
    useMusicLibraryStore.getState().invalidateYtMusic();
    release(status(true));
    await new Promise((r) => setTimeout(r, 10));

    // The superseded run must not have listed, nor written its results.
    expect(ytmusicAllTracks).not.toHaveBeenCalled();
    expect(useMusicLibraryStore.getState().ytmusic).toEqual([]);
    expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("idle");
  });

  it("is a no-op while a load is already running", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks.mockResolvedValue(page([track()], false));

    useMusicLibraryStore.getState().ensureYtMusic();
    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() => expect(settled()).toBe(true));

    expect(ytmusicStatus).toHaveBeenCalledTimes(1);
  });

  it("marks the source errored when the first listing fails", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks.mockRejectedValue(new Error("nope"));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("error"),
    );
    expect(useMusicLibraryStore.getState().ytmusic).toEqual([]);
  });

  // The account is signed in; only the *listing* failed. Reporting that as
  // signed-out made the Library show "Not signed in to YouTube Music" and send
  // the user to a Settings panel that correctly said they already were.
  it("keeps the account signed in when the listing fails, and records why", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks.mockRejectedValue(new Error("Could not load playlists: boom"));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("error"),
    );
    expect(useMusicLibraryStore.getState().ytmusicSignedIn).toBe(true);
    expect(useMusicLibraryStore.getState().ytmusicError).toBe(
      "Could not load playlists: boom",
    );
  });

  it("reports a signed-out account as signed out, with no error", async () => {
    ytmusicStatus.mockResolvedValue(status(false));

    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("ready"),
    );
    expect(useMusicLibraryStore.getState().ytmusicSignedIn).toBe(false);
    expect(useMusicLibraryStore.getState().ytmusicError).toBeNull();
    expect(ytmusicAllTracks).not.toHaveBeenCalled();
  });

  it("clears a previous error once a retry succeeds", async () => {
    ytmusicStatus.mockResolvedValue(status(true));
    ytmusicAllTracks.mockRejectedValue(new Error("boom"));
    useMusicLibraryStore.getState().ensureYtMusic();
    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusicError).toBe("boom"),
    );

    // What the Retry button does: drop the failed load, then re-run it. Served
    // fresh (not from cache) so there's no phase-2 background refresh to race.
    ytmusicAllTracks.mockResolvedValue(page([track()], false));
    useMusicLibraryStore.getState().invalidateYtMusic();
    useMusicLibraryStore.getState().ensureYtMusic();

    await vi.waitFor(() =>
      expect(useMusicLibraryStore.getState().ytmusicLoad).toBe("ready"),
    );
    expect(useMusicLibraryStore.getState().ytmusicError).toBeNull();
    expect(useMusicLibraryStore.getState().ytmusic).toHaveLength(1);
  });
});
