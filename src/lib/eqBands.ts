/**
 * Variable-band-count EQ as a *view* over the fixed 31-band engine.
 *
 * The DSP, presets, imports and stored state are all 31 ISO bands
 * ({@link ISO_CENTERS_HZ}) and never change. The UI can show a different number
 * of faders (5/10/16/20/31/32): each fader is a control point at a log-spaced
 * frequency, and its value is interpolated (linearly in dB, over log-frequency —
 * the same mapping AutoEQ / preset curves use) onto the 31 engine bands. Reading
 * back samples the engine curve at those same frequencies, so switching band
 * count preserves the sound.
 */
import { ISO_CENTERS_HZ, BAND_COUNT } from "./types";

/** Band counts the UI offers. 31 is the engine's native resolution. */
export const EQ_BAND_COUNTS = [5, 10, 16, 20, 31, 32] as const;
export type EqBandCount = (typeof EQ_BAND_COUNTS)[number];

const LO_HZ = 20;
const HI_HZ = 20_000;

/**
 * The `n` fader frequencies. For 31 this is exactly the engine's ISO centers
 * (so that view is lossless); otherwise `n` frequencies spaced evenly in
 * log-frequency across 20 Hz–20 kHz.
 */
export function bandFrequencies(n: number): number[] {
  if (n === BAND_COUNT) return [...ISO_CENTERS_HZ];
  if (n <= 1) return [Math.round(Math.sqrt(LO_HZ * HI_HZ))];
  const ratio = HI_HZ / LO_HZ;
  return Array.from({ length: n }, (_, i) =>
    Math.round(LO_HZ * Math.pow(ratio, i / (n - 1))),
  );
}

interface Anchor {
  hz: number;
  db: number;
}

/** Gain at `hz` from sorted `anchors`, linear in dB over log-frequency; flat
 *  (held) beyond the first/last anchor. */
function sampleAt(anchors: Anchor[], hz: number): number {
  if (anchors.length === 0) return 0;
  const x = Math.log10(hz);
  const first = anchors[0]!;
  const last = anchors[anchors.length - 1]!;
  if (x <= Math.log10(first.hz)) return first.db;
  if (x >= Math.log10(last.hz)) return last.db;
  for (let i = 1; i < anchors.length; i++) {
    const a = anchors[i - 1]!;
    const b = anchors[i]!;
    const xb = Math.log10(b.hz);
    if (x <= xb) {
      const xa = Math.log10(a.hz);
      const t = (x - xa) / (xb - xa);
      return a.db + t * (b.db - a.db);
    }
  }
  return last.db;
}

/** Sample the 31-band engine curve at `freqs` → one gain per fader (display). */
export function sampleEngineAt(bands31: readonly number[], freqs: number[]): number[] {
  const anchors: Anchor[] = ISO_CENTERS_HZ.map((hz, i) => ({
    hz,
    db: bands31[i] ?? 0,
  }));
  return freqs.map((hz) => sampleAt(anchors, hz));
}

/** Interpolate `n` fader control points (`freqs`/`gains`) onto the 31 engine
 *  bands. When `freqs` are the ISO centers (the 31-band view) this is exact. */
export function faderPointsToBands(freqs: number[], gains: number[]): number[] {
  const anchors: Anchor[] = freqs.map((hz, i) => ({ hz, db: gains[i] ?? 0 }));
  return ISO_CENTERS_HZ.map((hz) => {
    const db = sampleAt(anchors, hz);
    // Keep the engine's stored precision tidy (0.1 dB), matching the faders.
    return Math.round(db * 10) / 10;
  });
}
