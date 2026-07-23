import { describe, expect, it } from "vitest";
import { itemMeta, radioItem, reconcileDuration, ytmusicItem } from "@/stores/engine";
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
  hasVideo: false,
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

describe("radioItem", () => {
  it("is a ytmusic queue item marked auto-added — the queue UI badges it", () => {
    expect(radioItem(track())).toEqual({ ...ytmusicItem(track()), autoAdded: true });
  });
});

describe("reconcileDuration", () => {
  /** The bug this exists to fix: a streamed track's engine-reported total
   *  grows from ~1s as it downloads/decodes. Naively adopting it every tick
   *  made the seek bar's total visibly count up on every streamed track. */
  it("trusts the item's known duration while the engine's total is still a growing partial", () => {
    expect(reconcileDuration(210, 1, null)).toBe(210);
    expect(reconcileDuration(210, 90, null)).toBe(210);
    expect(reconcileDuration(210, 209, null)).toBe(210);
  });

  it("adopts the engine's total once it reaches or exceeds the item's", () => {
    expect(reconcileDuration(210, 210, null)).toBe(210);
    expect(reconcileDuration(210, 211, null)).toBe(211);
  });

  it("uses the engine's total when the item never had one (a local file, or unknown tags)", () => {
    expect(reconcileDuration(null, 180, null)).toBe(180);
  });

  it("falls back to the previous duration when neither the item nor this tick's engine value has one", () => {
    expect(reconcileDuration(null, null, 180)).toBe(180);
    expect(reconcileDuration(null, null, null)).toBeNull();
  });

  it("keeps a settled duration if a later tick's engine value briefly drops out (item has none of its own)", () => {
    // Mirrors calling reconcileDuration tick-over-tick with the previous
    // return value threaded back in as `prevDuration`.
    const afterEngineReports = reconcileDuration(null, 180, null);
    expect(afterEngineReports).toBe(180);
    expect(reconcileDuration(null, null, afterEngineReports)).toBe(180);
  });
});
