export type NetworkMode = "unknown" | "fast" | "constrained";

/** Throughput at/above this (bytes/sec ≈ 3.2 Mbps) comfortably prefetches a
 *  next track during the current one → safe for gapless. */
const FAST_BPS = 400_000;

/** Update the session network classification from one progress sample. A
 *  rebuffer always means constrained; sustained high throughput means fast —
 *  and lets a previously-constrained link *recover* to fast so it isn't stuck
 *  on the single-track path forever once the network improves. */
export function classify(
  prev: NetworkMode,
  sample: { downloadBps: number; rebufferDelta: number },
): NetworkMode {
  if (sample.rebufferDelta > 0) return "constrained";
  if (sample.downloadBps >= FAST_BPS) return "fast"; // recovers even from constrained
  if (prev === "constrained") return "constrained"; // stay cautious until proven fast
  return prev; // not enough evidence yet
}

/** Pick the playback mode for a streamed (cloud/phone/YouTube Music) queue.
 *
 *  Optimistic by default: crossfade/gapless is used unless the link has
 *  actually proven it can't keep up (a rebuffer this session → "constrained")
 *  or the user turned on Data Saver. An unmeasured ("unknown") link therefore
 *  gets crossfade immediately — matching local playback, which has no network
 *  gate at all. This is safe because the stream queue degrades to a plain hard
 *  cut (never a stall) when a lookahead track isn't ready in time, so being
 *  optimistic can only cost a missed crossfade, never wedge playback. */
export function chooseStreamMode(
  source: "cloud" | "phone" | "ytmusic",
  dataSaver: boolean,
  net: NetworkMode,
): "gapless" | "progressive" {
  void source;
  if (dataSaver) return "progressive";
  return net === "constrained" ? "progressive" : "gapless";
}

const LS_KEY = "hm.networkMode";
/** A classification older than this is treated as unknown so a link that has
 *  since recovered gets an optimistic (gapless) retry instead of being pinned
 *  to "constrained" forever when it never emits another throughput sample. */
const STALE_MS = 60 * 60 * 1000; // 1 hour

/** Restore the last measured network classification across app restarts and
 *  new queues so a proven-slow link stays on the single-track path (and a
 *  proven-fast one keeps crossfading) without re-paying the measurement cost
 *  each queue. Falls back to "unknown" (optimistic) when absent or stale. */
export function loadNetworkMode(): NetworkMode {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return "unknown";
    const parsed = JSON.parse(raw) as { mode?: unknown; at?: unknown };
    const mode = parsed.mode;
    const at = parsed.at;
    if (mode !== "fast" && mode !== "constrained") return "unknown";
    if (typeof at !== "number" || !Number.isFinite(at) || Date.now() - at > STALE_MS)
      return "unknown";
    return mode;
  } catch {
    return "unknown";
  }
}

/** Persist the current classification (stamped with the time) so it survives
 *  restarts and carries across queues. "unknown" clears the stored value. */
export function saveNetworkMode(mode: NetworkMode): void {
  try {
    if (mode === "unknown") localStorage.removeItem(LS_KEY);
    else localStorage.setItem(LS_KEY, JSON.stringify({ mode, at: Date.now() }));
  } catch {
    // Private mode / no storage — the classification just won't persist.
  }
}
