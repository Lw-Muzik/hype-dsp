import { describe, expect, it } from "vitest";
import { channelHealth, filterChannels, rankByHealth } from "./tvList";
import type { TvChannel } from "@/lib/types";

const ch = (id: string, name: string, group?: string): TvChannel => ({
  id,
  name,
  url: `http://x/${id}.m3u8`,
  logo: null,
  group: group ?? null,
  country: null,
  userAgent: null,
  referrer: null,
  quality: null,
});

describe("channelHealth", () => {
  const probed = new Set(["a", "b"]);
  const alive = new Set(["a"]);

  it("is dead only for a channel that was probed and failed", () => {
    expect(channelHealth("a", probed, alive)).toBe("alive");
    expect(channelHealth("b", probed, alive)).toBe("dead");
  });

  /** The distinction the dimming depends on: an unprobed channel is unknown, not
   *  dead — we must never dim a channel we never checked. */
  it("is unknown for a channel that was never probed", () => {
    expect(channelHealth("c", probed, alive)).toBe("unknown");
  });
});

describe("filterChannels", () => {
  const list = [ch("1", "BBC News", "News"), ch("2", "MTV", "Music"), ch("3", "CNN")];

  it("returns everything for an empty query", () => {
    expect(filterChannels(list, "  ").map((c) => c.id)).toEqual(["1", "2", "3"]);
  });

  it("matches name case-insensitively", () => {
    expect(filterChannels(list, "bbc").map((c) => c.id)).toEqual(["1"]);
  });

  it("matches the category too", () => {
    expect(filterChannels(list, "music").map((c) => c.id)).toEqual(["2"]);
  });
});

describe("rankByHealth", () => {
  const list = [ch("dead1", "D1"), ch("alive1", "A1"), ch("unk1", "U1"), ch("dead2", "D2"), ch("alive2", "A2")];
  const probed = new Set(["dead1", "alive1", "dead2", "alive2"]);
  const alive = new Set(["alive1", "alive2"]);

  it("orders alive, then unknown, then dead", () => {
    expect(rankByHealth(list, probed, alive).map((c) => c.id)).toEqual([
      "alive1",
      "alive2",
      "unk1",
      "dead1",
      "dead2",
    ]);
  });

  /** Stable within a band: alive1 stays before alive2 (their catalog order),
   *  dead1 before dead2. A reshuffle inside a band would be visible churn. */
  it("preserves catalog order within each health band", () => {
    const out = rankByHealth(list, probed, alive).map((c) => c.id);
    expect(out.indexOf("alive1")).toBeLessThan(out.indexOf("alive2"));
    expect(out.indexOf("dead1")).toBeLessThan(out.indexOf("dead2"));
  });

  /** Before any probe completes, the list must not reorder — otherwise it jumps
   *  the instant a check returns nothing. */
  it("leaves the list untouched when nothing has been probed", () => {
    const out = rankByHealth(list, new Set(), new Set());
    expect(out.map((c) => c.id)).toEqual(list.map((c) => c.id));
  });
});
