import type { MovieStream } from "@/lib/types";

export type Quality = "SD" | "HD";

/** SD first — the same default order `movies_resolve` builds `streams` in. */
const QUALITY_ORDER: readonly Quality[] = ["SD", "HD"];

/**
 * Which stream to play for a chosen quality.
 *
 * Falls back to whichever stream the movie actually has when the requested
 * label isn't among them (an item that only offers one rendition — the
 * backend's own SD-borrows-HD fallback — still has *something* to play), and
 * to `null` only when there is truly nothing playable.
 */
export function selectStream(streams: MovieStream[], quality: Quality): MovieStream | null {
  return streams.find((s) => s.label === quality) ?? streams[0] ?? null;
}

/**
 * Which qualities this movie actually offers, SD-then-HD. Used to decide
 * whether an SD/HD toggle is worth showing at all — a single-rendition item
 * has no real choice to offer.
 */
export function availableQualities(streams: MovieStream[]): Quality[] {
  const labels = new Set(streams.map((s) => s.label));
  return QUALITY_ORDER.filter((q) => labels.has(q));
}
