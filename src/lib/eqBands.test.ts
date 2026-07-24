import { describe, expect, it } from "vitest";
import { ISO_CENTERS_HZ, BAND_COUNT } from "./types";
import {
  EQ_BAND_COUNTS,
  bandFrequencies,
  faderPointsToBands,
  sampleEngineAt,
} from "./eqBands";

describe("eqBands", () => {
  it("uses the exact ISO centers for the 31-band view", () => {
    expect(bandFrequencies(31)).toEqual([...ISO_CENTERS_HZ]);
  });

  it("spans 20 Hz–20 kHz for every count", () => {
    for (const n of EQ_BAND_COUNTS) {
      const f = bandFrequencies(n);
      expect(f).toHaveLength(n);
      expect(f[0]).toBe(20);
      expect(f[f.length - 1]).toBe(20_000);
      // Strictly increasing (log-spaced).
      for (let i = 1; i < f.length; i++) expect(f[i]!).toBeGreaterThan(f[i - 1]!);
    }
  });

  it("is lossless at 31 bands (ISO centers → engine is identity)", () => {
    const gains = ISO_CENTERS_HZ.map((_, i) => (i % 2 === 0 ? 3 : -2));
    const freqs = bandFrequencies(31);
    const back = faderPointsToBands(freqs, gains);
    expect(back).toEqual(gains);
    expect(back).toHaveLength(BAND_COUNT);
  });

  it("preserves a flat curve through any band count", () => {
    const flat = new Array(BAND_COUNT).fill(0);
    for (const n of EQ_BAND_COUNTS) {
      const freqs = bandFrequencies(n);
      const faders = sampleEngineAt(flat, freqs); // all 0
      expect(faders.every((v) => v === 0)).toBe(true);
      const engine = faderPointsToBands(freqs, faders);
      expect(engine.every((v) => v === 0)).toBe(true);
    }
  });

  it("keeps a monotonic (bass-flat, treble-boosted) shape when down-sampled", () => {
    // A rising staircase like the screenshot: 0 dB below ~700 Hz, ramping up.
    const engine = ISO_CENTERS_HZ.map((hz) => (hz < 700 ? 0 : Math.min(12, (hz - 700) / 1600)));
    const freqs = bandFrequencies(10);
    const faders = sampleEngineAt(engine, freqs);
    // Non-decreasing across the 10 faders (treble never dips below bass).
    for (let i = 1; i < faders.length; i++) {
      expect(faders[i]!).toBeGreaterThanOrEqual(faders[i - 1]! - 1e-6);
    }
    // The low fader is ~flat, the top fader is boosted.
    expect(faders[0]!).toBeCloseTo(0, 1);
    expect(faders[faders.length - 1]!).toBeGreaterThan(4);
  });
});
