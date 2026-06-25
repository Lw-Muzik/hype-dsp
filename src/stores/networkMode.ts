export type NetworkMode = "unknown" | "fast" | "constrained";

/** Throughput at/above this (bytes/sec ≈ 3.2 Mbps) comfortably prefetches a
 *  next track during the current one → safe for gapless. */
const FAST_BPS = 400_000;

/** Update the session network classification from one progress sample. A
 *  rebuffer always means constrained; sustained high throughput means fast. */
export function classify(
  prev: NetworkMode,
  sample: { downloadBps: number; rebufferDelta: number },
): NetworkMode {
  if (sample.rebufferDelta > 0) return "constrained";
  if (prev === "constrained") return "constrained"; // sticky until a new queue
  if (sample.downloadBps >= FAST_BPS) return "fast";
  return prev; // not enough evidence yet
}

/** Pick the playback mode for a streamed (cloud/phone) queue. */
export function chooseStreamMode(
  source: "cloud" | "phone",
  dataSaver: boolean,
  net: NetworkMode,
): "gapless" | "progressive" {
  void source;
  if (dataSaver) return "progressive";
  return net === "fast" ? "gapless" : "progressive";
}
