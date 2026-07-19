import type { TvChannel } from "@/lib/types";

/**
 * The presentation logic for a TV channel list — filtering and health-ordering —
 * as pure functions, so the part that decides what a user sees is tested without
 * a component, a network, or the DOM.
 */

/** What a health probe concluded about a channel, from the caller's sets.
 *
 *  A channel is only `dead` if it was actually probed and failed. One we haven't
 *  probed is `unknown`, never `dead` — we don't dim a channel we never checked. */
export type Health = "alive" | "dead" | "unknown";

export function channelHealth(
  id: string,
  probed: ReadonlySet<string>,
  alive: ReadonlySet<string>,
): Health {
  if (!probed.has(id)) return "unknown";
  return alive.has(id) ? "alive" : "dead";
}

/** Case-insensitive filter over a channel's name and category.
 *
 *  For filtering *within* a loaded country/category list — distinct from the
 *  global catalog search, which is a backend call. An empty query is the whole
 *  list. */
export function filterChannels(channels: TvChannel[], query: string): TvChannel[] {
  const q = query.trim().toLowerCase();
  if (!q) return channels;
  return channels.filter(
    (c) =>
      c.name.toLowerCase().includes(q) ||
      (c.group?.toLowerCase().includes(q) ?? false),
  );
}

/** Working channels first, unchecked next, dead last — a **stable** reorder, so
 *  within each band the catalog's own order is preserved.
 *
 *  Dead channels are kept, not dropped: a stream that's momentarily down should
 *  sink, not vanish (and reappear jarringly when it recovers). The caller dims
 *  them. Returns the input unchanged when nothing has been probed yet, so the
 *  list doesn't reshuffle the instant a check completes on an empty result. */
export function rankByHealth(
  channels: TvChannel[],
  probed: ReadonlySet<string>,
  alive: ReadonlySet<string>,
): TvChannel[] {
  if (probed.size === 0) return channels;
  const rank: Record<Health, number> = { alive: 0, unknown: 1, dead: 2 };
  // Decorate-sort-undecorate keeps it stable and O(n log n) without relying on
  // the engine's sort stability for the tie-break — the index is the tie-break.
  return channels
    .map((c, i) => ({ c, i, r: rank[channelHealth(c.id, probed, alive)] }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.c);
}
