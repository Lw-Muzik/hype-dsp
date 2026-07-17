export type NetworkMode = "unknown" | "fast" | "constrained";

/** Throughput at/above this (bytes/sec ≈ 3.2 Mbps) comfortably prefetches a
 *  next track during the current one → safe for gapless. */
const FAST_BPS = 400_000;

/** Rebuffers needed before a link is called constrained.
 *
 *  One is not evidence. The cushion ahead of the decoder is two seconds, so a
 *  single momentary drain is the cheapest thing a healthy link can do — a GC
 *  pause, a CDN hiccup, a Wi-Fi roam. Demoting on one costs every later track
 *  its crossfade to buy nothing. */
const STRIKES_TO_CONSTRAIN = 3;

/** How long a "constrained" verdict stands before the link is re-tried.
 *
 *  It has to expire *on a clock*, because it cannot expire on evidence. The
 *  throughput a constrained link would need to report to clear itself is never
 *  measured: the download is backpressured to playback rate (the reader sleeps
 *  once the ring is full), so the figure that comes back is roughly the track's
 *  own bitrate — tens of KB/s for m4a, two orders of magnitude below `FAST_BPS`.
 *  A verdict whose only exit is a number that cannot occur is not a
 *  classification, it's a trapdoor: one rebuffer and gapless was gone for good.
 *
 *  So recovery is time, not proof. Ten minutes is short enough that a passing
 *  problem doesn't outlive itself, and re-testing costs only a hard cut if the
 *  link is still bad — which is the same thing being wrong optimistically has
 *  always cost here. */
const CONSTRAINED_TTL_MS = 10 * 60 * 1000;

/** What we've learned about the link, with when we learned it.
 *
 *  A mode alone can't be aged, and an unageable "constrained" is permanent. */
export interface NetworkState {
  mode: NetworkMode;
  /** Epoch ms when `mode` was last decided. */
  at: number;
  /** Rebuffers seen since the last verdict. */
  strikes: number;
}

export const UNKNOWN_NETWORK: NetworkState = { mode: "unknown", at: 0, strikes: 0 };

/** Fold one progress sample into what we know about the link. */
export function observe(
  prev: NetworkState,
  sample: { downloadBps: number; rebufferDelta: number },
  now: number,
): NetworkState {
  if (sample.rebufferDelta > 0) {
    const strikes = prev.strikes + sample.rebufferDelta;
    if (strikes < STRIKES_TO_CONSTRAIN) return { ...prev, strikes };
    return { mode: "constrained", at: now, strikes: 0 };
  }
  // Only ever reachable on a link whose throughput is measured off the critical
  // path; kept because it is the one *evidenced* route back to fast, and it
  // costs nothing when it doesn't fire.
  if (sample.downloadBps >= FAST_BPS) return { mode: "fast", at: now, strikes: 0 };
  return prev;
}

/** The verdict as it stands *now* — constrained decays back to unknown.
 *
 *  Evaluated per decision rather than once at startup: a classification read
 *  only at launch cannot expire during the session it is punishing. */
export function effectiveMode(state: NetworkState, now: number): NetworkMode {
  if (state.mode === "constrained" && now - state.at > CONSTRAINED_TTL_MS) return "unknown";
  return state.mode;
}

/** Pick the playback mode for a streamed (cloud/phone/YouTube Music) queue.
 *
 *  Optimistic by default: crossfade/gapless is used unless the link has actually
 *  proven it can't keep up, or the user turned on Data Saver. An unmeasured
 *  ("unknown") link therefore gets crossfade immediately — matching local
 *  playback, which has no network gate at all. This is safe because the stream
 *  queue degrades to a plain hard cut (never a stall) when a lookahead track
 *  isn't ready in time, so being optimistic can only cost a missed crossfade,
 *  never wedge playback. */
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
/** A stored verdict older than this is discarded at launch. Longer than
 *  `CONSTRAINED_TTL_MS`, which governs the live decay; this only keeps a link
 *  that was proven fast from being re-measured every cold start. */
const STALE_MS = 60 * 60 * 1000; // 1 hour

/** Restore the last classification across restarts so a proven-fast link keeps
 *  crossfading without re-paying the measurement. Falls back to "unknown"
 *  (optimistic) when absent or stale. */
export function loadNetworkMode(): NetworkState {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return UNKNOWN_NETWORK;
    const parsed = JSON.parse(raw) as { mode?: unknown; at?: unknown };
    const mode = parsed.mode;
    const at = parsed.at;
    if (mode !== "fast" && mode !== "constrained") return UNKNOWN_NETWORK;
    if (typeof at !== "number" || !Number.isFinite(at) || Date.now() - at > STALE_MS)
      return UNKNOWN_NETWORK;
    return { mode, at, strikes: 0 };
  } catch {
    return UNKNOWN_NETWORK;
  }
}

/** Persist the current classification (stamped with the time) so it survives
 *  restarts and carries across queues. "unknown" clears the stored value. */
export function saveNetworkMode(state: NetworkState): void {
  try {
    if (state.mode === "unknown") localStorage.removeItem(LS_KEY);
    else localStorage.setItem(LS_KEY, JSON.stringify({ mode: state.mode, at: state.at }));
  } catch {
    // Private mode / no storage — the classification just won't persist.
  }
}
