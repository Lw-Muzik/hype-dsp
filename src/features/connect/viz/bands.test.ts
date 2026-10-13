import { describe, expect, it } from "vitest";
import {
  bassBandCount,
  bassDb,
  clamp01,
  dbToUnit,
  rebinBands,
  round2,
  SOURCE_BANDS,
  SOURCE_HIGH_HZ,
  SOURCE_LOW_HZ,
  unitToDb,
  VIZ_BAND_COUNT,
  VIZ_MAX_HZ,
  VIZ_MIN_HZ,
  vizLevel,
} from "./bands";

const zeros = () => new Array<number>(SOURCE_BANDS).fill(0);
const filled = (v: number) => new Array<number>(SOURCE_BANDS).fill(v);
const sourceIndexAt = (hz: number, n = SOURCE_BANDS, high = SOURCE_HIGH_HZ) =>
  Math.floor((n * Math.log(hz / SOURCE_LOW_HZ)) / Math.log(high / SOURCE_LOW_HZ));
const targetIndexAt = (hz: number) =>
  Math.floor((VIZ_BAND_COUNT * Math.log(hz / VIZ_MIN_HZ)) / Math.log(VIZ_MAX_HZ / VIZ_MIN_HZ));

describe("dB mapping", () => {
  it("maps -80 dBFS to 0 and 0 dBFS to 1, linearly", () => {
    expect(dbToUnit(-80)).toBe(0);
    expect(dbToUnit(0)).toBe(1);
    expect(dbToUnit(-40)).toBe(0.5);
    expect(dbToUnit(-20)).toBeCloseTo(0.75, 10);
  });

  it("clamps outside the range and reads NaN as silence", () => {
    expect(dbToUnit(-120)).toBe(0);
    expect(dbToUnit(Number.NEGATIVE_INFINITY)).toBe(0);
    expect(dbToUnit(6)).toBe(1);
    expect(dbToUnit(Number.NaN)).toBe(0);
  });

  it("unitToDb inverts it inside 0..1 and clamps outside", () => {
    for (const v of [0, 0.25, 0.5, 1]) expect(dbToUnit(unitToDb(v))).toBeCloseTo(v, 10);
    expect(unitToDb(-1)).toBe(-80);
    expect(unitToDb(2)).toBe(0);
  });

  it("clamp01 and round2", () => {
    expect(clamp01(-0.1)).toBe(0);
    expect(clamp01(1.1)).toBe(1);
    expect(clamp01(Number.NaN)).toBe(0);
    expect(clamp01(Number.POSITIVE_INFINITY)).toBe(0);
    expect(clamp01(0.3)).toBe(0.3);
    expect(round2(0.123456)).toBe(0.12);
    expect(round2(0.999)).toBe(1);
  });
});

describe("rebinBands", () => {
  it("always returns the contract's 32 bands", () => {
    expect(rebinBands(zeros())).toHaveLength(32);
    expect(rebinBands([])).toEqual(new Array(32).fill(0));
    expect(rebinBands([0.5])).toHaveLength(32);
  });

  it("keeps silence silent and a flat spectrum flat", () => {
    expect(rebinBands(zeros())).toEqual(new Array(32).fill(0));
    expect(rebinBands(filled(0.5))).toEqual(new Array(32).fill(0.5));
    expect(rebinBands(filled(1))).toEqual(new Array(32).fill(1));
  });

  it("puts a 1 kHz tone in the 1 kHz band and nowhere far from it", () => {
    const s = zeros();
    s[sourceIndexAt(1000)] = 0.9;
    const out = rebinBands(s);
    const at = targetIndexAt(1000);
    expect(out[at]).toBe(0.9);
    // Spreads at most into the neighbour a wide source band also touches.
    expect(out.filter((v) => v > 0).length).toBeLessThanOrEqual(2);
    for (let i = 0; i < out.length; i++) {
      if (Math.abs(i - at) > 1) expect(out[i], `band ${i}`).toBe(0);
    }
  });

  it("keeps low-first order: bass lights the low bands, treble the high ones", () => {
    const bass = zeros();
    bass[sourceIndexAt(60)] = 1;
    const treble = zeros();
    treble[sourceIndexAt(8000)] = 1;
    const b = rebinBands(bass);
    const t = rebinBands(treble);
    expect(b.indexOf(1)).toBeLessThan(6);
    expect(t.indexOf(1)).toBeGreaterThan(24);
  });

  it("takes the loudest source band under a target (a bin's peak, not an average)", () => {
    const s = zeros();
    const k = sourceIndexAt(1000);
    s[k] = 0.4;
    s[k + 1] = 0.8;
    const out = rebinBands(s);
    expect(Math.max(...out)).toBe(0.8);
  });

  it("ignores what lies below 30 Hz and above 16 kHz", () => {
    const sub = zeros();
    sub[0] = 1; // 20-21.4 Hz
    expect(rebinBands(sub)).toEqual(new Array(32).fill(0));
    const air = zeros();
    for (let k = sourceIndexAt(17_000); k < SOURCE_BANDS; k++) air[k] = 1;
    expect(rebinBands(air)).toEqual(new Array(32).fill(0));
  });

  it("lights a target only for a source band that overlaps it by a quarter of its width", () => {
    const srcW = Math.log(SOURCE_HIGH_HZ / SOURCE_LOW_HZ) / SOURCE_BANDS;
    const tgtW = Math.log(VIZ_MAX_HZ / VIZ_MIN_HZ) / VIZ_BAND_COUNT;
    let slivers = 0;
    for (let k = 0; k < SOURCE_BANDS; k++) {
      const s = zeros();
      s[k] = 1;
      const out = rebinBands(s);
      const a = Math.log(SOURCE_LOW_HZ) + k * srcW;
      for (let i = 0; i < VIZ_BAND_COUNT; i++) {
        const from = Math.log(VIZ_MIN_HZ) + i * tgtW;
        const overlap = Math.min(from + tgtW, a + srcW) - Math.max(from, a);
        if (overlap > 0 && overlap < 0.25 * srcW) {
          slivers += 1;
          expect(out[i], `source ${k} only grazes target ${i}`).toBe(0);
        }
        if (overlap >= 0.25 * srcW) expect(out[i], `source ${k} -> target ${i}`).toBe(1);
      }
      // …and every audible source band lights at least one target in range.
      const centreHz = SOURCE_LOW_HZ * (SOURCE_HIGH_HZ / SOURCE_LOW_HZ) ** ((k + 0.5) / SOURCE_BANDS);
      if (centreHz > VIZ_MIN_HZ * 1.2 && centreHz < VIZ_MAX_HZ / 1.2) {
        expect(Math.max(...out), `source ${k} (${centreHz.toFixed(0)} Hz) vanished`).toBe(1);
      }
    }
    expect(slivers, "the geometry has grazing overlaps, or this test proves nothing").toBeGreaterThan(5);
  });

  it("clamps out-of-range input and reads NaN as silence", () => {
    const s = zeros();
    s[sourceIndexAt(1000)] = 5;
    s[sourceIndexAt(100)] = Number.NaN;
    s[sourceIndexAt(4000)] = -3;
    const out = rebinBands(s);
    expect(Math.max(...out)).toBe(1);
    expect(Math.min(...out)).toBe(0);
    expect(out.every((v) => v >= 0 && v <= 1 && Number.isFinite(v))).toBe(true);
  });

  it("rounds to two decimals like the contract's examples", () => {
    expect(rebinBands(filled(0.123456))[10]).toBe(0.12);
  });

  it("silences targets above a low sample rate's top band", () => {
    // 22.05 kHz Nyquist -> the source tops out at ~10.9 kHz.
    const out = rebinBands(filled(1), 10_900);
    expect(out[VIZ_BAND_COUNT - 1]).toBe(0);
    expect(out[0]).toBe(1);
  });

  it("copes with a different source resolution", () => {
    const out = rebinBands(new Array(16).fill(0.7));
    expect(out).toEqual(new Array(32).fill(0.7));
  });
});

describe("vizLevel", () => {
  it("is 0 for silence or nothing, 1 at full scale, and follows a flat level", () => {
    expect(vizLevel([])).toBe(0);
    expect(vizLevel(zeros())).toBe(0);
    expect(vizLevel(filled(1))).toBe(1);
    expect(vizLevel(filled(0.5))).toBe(0.5);
  });

  it("is the MEAN power, not the sum or the max", () => {
    const one = zeros();
    one[10] = 1; // one band at 0 dBFS, 63 at -80
    // mean power = 1/64 -> -18.06 dBFS -> 0.774
    expect(vizLevel(one)).toBe(0.77);
  });

  it("clamps garbage", () => {
    expect(vizLevel(filled(7))).toBe(1);
    expect(vizLevel(filled(Number.NaN))).toBe(0);
  });
});

describe("bass", () => {
  it("covers the bands below 150 Hz", () => {
    expect(bassBandCount()).toBe(19);
    expect(bassBandCount(32)).toBeLessThan(bassBandCount());
    expect(bassBandCount(1)).toBe(1);
  });

  it("is the mean power of those bands in dBFS", () => {
    const s = zeros();
    for (let k = 0; k < 19; k++) s[k] = 0.5; // -40 dBFS
    expect(bassDb(s)).toBeCloseTo(-40, 6);
    s[40] = 1; // outside the bass: ignored
    expect(bassDb(s)).toBeCloseTo(-40, 6);
  });

  it("is the floor for silence or no data", () => {
    expect(bassDb(zeros())).toBeCloseTo(-80, 6);
    expect(bassDb([])).toBe(-80);
  });
});
