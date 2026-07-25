import { describe, expect, it } from "vitest";
import { availableQualities, selectStream } from "@/features/movies/streamSelect";
import type { MovieStream } from "@/lib/types";

const sd: MovieStream = { label: "SD", url: "https://archive.org/download/x/sd.mp4", mime: "video/mp4" };
const hd: MovieStream = { label: "HD", url: "https://archive.org/download/x/hd.mp4", mime: "video/mp4" };

describe("selectStream", () => {
  it("picks the stream matching the requested quality", () => {
    expect(selectStream([sd, hd], "SD")).toEqual(sd);
    expect(selectStream([sd, hd], "HD")).toEqual(hd);
  });

  it("falls back to whichever stream exists when the requested quality is missing", () => {
    // Mirrors the backend's own SD-borrows-HD fallback: an item with only one
    // rendition still has something to play under either toggle position.
    expect(selectStream([hd], "SD")).toEqual(hd);
    expect(selectStream([sd], "HD")).toEqual(sd);
  });

  it("returns null when there is truly nothing playable", () => {
    expect(selectStream([], "SD")).toBeNull();
    expect(selectStream([], "HD")).toBeNull();
  });
});

describe("availableQualities", () => {
  it("lists SD before HD regardless of the backend's own array order", () => {
    expect(availableQualities([hd, sd])).toEqual(["SD", "HD"]);
    expect(availableQualities([sd, hd])).toEqual(["SD", "HD"]);
  });

  it("lists just the one quality a single-rendition item has", () => {
    expect(availableQualities([hd])).toEqual(["HD"]);
    expect(availableQualities([sd])).toEqual(["SD"]);
  });

  it("is empty when there are no streams — the toggle has nothing to offer", () => {
    expect(availableQualities([])).toEqual([]);
  });
});
