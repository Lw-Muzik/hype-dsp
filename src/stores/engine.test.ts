import { describe, expect, it } from "vitest";
import { itemMeta, ytmusicItem } from "@/stores/engine";
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

describe("itemMeta", () => {
  const item = () => ytmusicItem(track());

  /** The sidebar's now-playing card renders `nowPlayingMeta.cover`. This used to
   *  hardcode null, so a YT Music track sat on its gradient forever: the engine
   *  can only supply art it decodes from the file, and googlevideo streams a
   *  bare DASH audio track with no embedded tags. The thumbnail was on the item
   *  the whole time. */
  it("carries the item's own cover", () => {
    expect(itemMeta(item()).cover).toBe("https://i.ytimg.com/vi/vid1/hq.jpg");
  });

  it("passes the item's tags through", () => {
    expect(itemMeta(item())).toEqual({
      title: "Song",
      artist: "Artist",
      album: "Album",
      cover: "https://i.ytimg.com/vi/vid1/hq.jpg",
    });
  });

  /** A local file has no cover on the item — its art is decoded from the file
   *  later, so null here is correct and must stay null (not undefined). */
  it("is null when the item has no cover of its own", () => {
    expect(itemMeta({ ...item(), cover: null }).cover).toBeNull();
    const { cover: _drop, ...noCover } = item();
    expect(itemMeta(noCover).cover).toBeNull();
  });
});
