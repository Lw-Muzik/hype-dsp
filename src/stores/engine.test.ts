import { describe, expect, it } from "vitest";
import { ytmusicItem } from "@/stores/engine";
import type { YtTrack } from "@/lib/types";

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

describe("ytmusicItem", () => {
  it("maps a track onto a ytmusic queue item", () => {
    expect(ytmusicItem(track())).toEqual({
      id: "vid1",
      source: "ytmusic",
      title: "Song",
      artist: "Artist",
      album: "Album",
      durationSecs: 210,
      cover: "https://i.ytimg.com/vi/vid1/hq.jpg",
      ytTrack: track(),
    });
  });

  it("keys the item by videoId — that's what playback resolves with", () => {
    expect(ytmusicItem(track({ videoId: "abc" })).id).toBe("abc");
  });

  it("carries the thumbnail as the cover, so the card needs no art fetch", () => {
    expect(ytmusicItem(track({ thumbnail: "https://x/y.jpg" })).cover).toBe(
      "https://x/y.jpg",
    );
    // No thumbnail must stay null (not undefined) — the Artwork gradient
    // fallback keys off a falsy cover.
    expect(ytmusicItem(track({ thumbnail: null })).cover).toBeNull();
  });

  it("passes null tags through rather than inventing placeholders", () => {
    const item = ytmusicItem(track({ artist: null, album: null, durationSecs: null }));
    expect(item.artist).toBeNull();
    expect(item.album).toBeNull();
    expect(item.durationSecs).toBeNull();
  });

  it("keeps the source payload so the queue can resolve the stream", () => {
    // startPlayback reads `ytTrack.videoId` off the queue, not `id`.
    expect(ytmusicItem(track()).ytTrack?.videoId).toBe("vid1");
  });
});
