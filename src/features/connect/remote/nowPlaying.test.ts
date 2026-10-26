import { describe, expect, it } from "vitest";
import { backdropSource } from "@/features/theme/backdropSource";
import type { TrackMeta } from "@/lib/types";
import { remoteMeta, remoteProgress, selectNowPlaying } from "./nowPlaying";
import type { RemoteView } from "./session";

const local: TrackMeta = { title: "Local", artist: "Me", album: "LP", cover: "data:local" };

function view(over: Partial<RemoteView> = {}): RemoteView {
  return {
    device: { id: "p", name: "Pixel", caps: ["player"] },
    seq: 1,
    track: { kind: "yt", videoId: "v", title: "Remote", artist: "Them", album: "EP", artUrl: "https://i/x.jpg", durationMs: 200_000 },
    playing: true,
    positionMs: 10_000,
    at: 1_000,
    durationMs: 200_000,
    volume: 1,
    shuffle: false,
    repeat: "off",
    queue: [],
    ...over,
  } as unknown as RemoteView;
}

describe("selectNowPlaying", () => {
  it("uses the local track when not in remote mode", () => {
    const np = selectNowPlaying(local, null);
    expect(np).toEqual({ remote: false, deviceName: null, meta: local });
  });

  it("uses the remote track (title, artist, art) in remote mode", () => {
    const np = selectNowPlaying(local, view());
    expect(np.remote).toBe(true);
    expect(np.deviceName).toBe("Pixel");
    expect(np.meta).toEqual({ title: "Remote", artist: "Them", album: "EP", cover: "https://i/x.jpg" });
  });

  it("remote with nothing playing shows nothing, not the stale local track", () => {
    expect(selectNowPlaying(local, view({ track: null })).meta).toBeNull();
  });

  it("a remote track change moves the backdrop source with it", () => {
    const a = backdropSource(selectNowPlaying(local, view()).meta);
    const next = view({ track: { ...view().track!, title: "B", artUrl: "https://i/y.jpg" } as never });
    const b = backdropSource(selectNowPlaying(local, next).meta);
    expect(a).toEqual({ kind: "art", url: "https://i/x.jpg" });
    expect(b).toEqual({ kind: "art", url: "https://i/y.jpg" });
  });

  it("a remote track without art falls back to the gradient, like the UI", () => {
    const m = remoteMeta(view({ track: { ...view().track!, artUrl: null } as never }));
    expect(m?.cover).toBeNull();
    expect(backdropSource(m)?.kind).toBe("gradient");
  });
});

describe("remoteProgress", () => {
  it("ticks with hub time while playing", () => {
    expect(remoteProgress(view(), 4_000)).toEqual({ position: 13, duration: 200 });
  });
  it("holds still while paused", () => {
    expect(remoteProgress(view({ playing: false }), 99_000).position).toBe(10);
  });
  it("clamps to the duration and to zero", () => {
    expect(remoteProgress(view(), 1_000_000).position).toBe(200);
    expect(remoteProgress(view({ positionMs: 0 }), 0).position).toBe(0);
  });
  it("falls back to the track duration when the state has none", () => {
    expect(remoteProgress(view({ durationMs: null }), 1_000).duration).toBe(200);
  });
});
