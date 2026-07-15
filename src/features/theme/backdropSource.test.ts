import { describe, expect, it } from "vitest";
import { backdropSource } from "@/features/theme/backdropSource";
import type { TrackMeta } from "@/lib/types";

const meta = (over: Partial<TrackMeta> = {}): TrackMeta => ({
  title: "Song", artist: "Artist", album: "Album", cover: null, ...over,
});

describe("backdropSource", () => {
  it("paints the cover when there is one", () => {
    expect(backdropSource(meta({ cover: "data:image/jpeg;base64,AAA" })))
      .toEqual({ kind: "art", url: "data:image/jpeg;base64,AAA" });
  });

  it("falls back to the same gradient Artwork shows", () => {
    // Not a placeholder colour: matching Artwork means the backdrop and the
    // on-screen cover agree for tracks with no embedded art.
    const got = backdropSource(meta({ album: "Kind of Blue" }));
    expect(got?.kind).toBe("gradient");
    expect(got).toEqual(backdropSource(meta({ album: "Kind of Blue" })));
  });

  it("seeds the gradient from album, falling back to title", () => {
    const byAlbum = backdropSource(meta({ album: "A", title: "T" }));
    const byTitle = backdropSource(meta({ album: null, title: "A" }));
    expect(byAlbum).toEqual(byTitle);
  });

  it("paints nothing when nothing is playing", () => {
    expect(backdropSource(null)).toBeNull();
  });
});
