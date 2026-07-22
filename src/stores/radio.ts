import type { QueueItem, RepeatMode } from "@/stores/engine";
import type { YtTrack } from "@/lib/types";

/**
 * The decision logic behind the endless queue, kept pure so it's testable.
 * The engine store owns the session and the fetches; this answers one
 * question: given where playback is, what — if anything — should radio do?
 */

/** A live radio session: the seed the queue grew from and where the next page
 *  continues. A `null` continuation means the chain broke — the next step
 *  re-seeds from the end of the queue instead of stopping. */
export interface RadioSession {
  seedId: string;
  continuation: string | null;
}

/** Fetch more once this few unplayed tracks remain ahead of the listener. */
export const RADIO_LOW_WATER = 5;

export type RadioStep =
  | { kind: "continue"; seedId: string; token: string }
  | { kind: "reseed"; seedId: string }
  | { kind: "start"; seedId: string }
  | null;

/** What radio should do now that playback sits at `orderPos`. */
export function radioStep(args: {
  autoplay: boolean;
  fetching: boolean;
  session: RadioSession | null;
  orderLen: number;
  orderPos: number;
  /** Whole queue is YT Music tracks — radio can only grow those. */
  allYtMusic: boolean;
  /** videoId of the last track in play order: the seed for extension. */
  lastVideoId: string | null;
  repeat: RepeatMode;
}): RadioStep {
  const { autoplay, fetching, session, orderLen, orderPos, allYtMusic, lastVideoId, repeat } = args;
  if (!autoplay || fetching || orderLen === 0 || orderPos < 0) return null;
  // Repeat loops the current list; against an ever-growing queue the loop
  // would never come round, so repeat wins over radio.
  if (repeat !== "off") return null;
  const remaining = orderLen - orderPos - 1;
  if (remaining > RADIO_LOW_WATER) return null;
  if (session) {
    if (session.continuation) {
      return { kind: "continue", seedId: session.seedId, token: session.continuation };
    }
    return lastVideoId ? { kind: "reseed", seedId: lastVideoId } : null;
  }
  if (!allYtMusic || !lastVideoId) return null;
  return { kind: "start", seedId: lastVideoId };
}

/** Incoming radio tracks not already in the queue (continuation pages
 *  overlap) and actually streamable, original order kept. */
export function dedupeRadioTracks(queue: QueueItem[], incoming: YtTrack[]): YtTrack[] {
  const seen = new Set(queue.map((q) => q.id));
  const out: YtTrack[] = [];
  for (const t of incoming) {
    if (!t.isAvailable || seen.has(t.videoId)) continue;
    seen.add(t.videoId);
    out.push(t);
  }
  return out;
}
