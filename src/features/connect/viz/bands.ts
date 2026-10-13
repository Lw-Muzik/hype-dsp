/**
 * From the engine's spectrum to the contract's `VizFrame` numbers (§3.11).
 *
 * # What the engine gives us
 *
 * `EngineFrame.spectrum` is NOT raw FFT bins: `hm-audio`'s `Analyzer`
 * (`crates/hm-audio/src/spectrum.rs`) already folds a 2048-point FFT (Hann
 * window) into {@link SOURCE_BANDS} log-spaced bands from {@link SOURCE_LOW_HZ}
 * to {@link SOURCE_HIGH_HZ} (20 kHz, or 0.99 x Nyquist below a 40 kHz sample
 * rate — the frame doesn't carry the rate, so 20 kHz is assumed), each the
 * loudest bin in its range, in dB, and maps it `-80 dBFS -> 0 .. 0 dBFS -> 1`,
 * clamped. That is the very mapping the contract wants, so no dB conversion is
 * needed on the way out: the work here is re-binning 64 bands over 20 Hz-20 kHz
 * onto 32 over 30 Hz-16 kHz, and measuring the level and the bass.
 */

export const SOURCE_BANDS = 64;
export const SOURCE_LOW_HZ = 20;
export const SOURCE_HIGH_HZ = 20_000;

export const VIZ_BAND_COUNT = 32;
export const VIZ_MIN_HZ = 30;
export const VIZ_MAX_HZ = 16_000;

/** dBFS that maps to 0 (0 dBFS maps to 1). */
export const VIZ_FLOOR_DB = -80;

/** Bass is what a kick and an 808 put below this. */
export const BASS_MAX_HZ = 150;

/** A source band must overlap a target by this share of its own width to
 *  count toward it, so a sliver at the edge doesn't bleed its neighbour in. */
const MIN_OVERLAP = 0.25;

export const round2 = (v: number): number => Math.round(v * 100) / 100;

/** A value clamped to 0..1; NaN and infinities are silence. */
export function clamp01(v: number): number {
  return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
}

/** dBFS -> 0..1 linearly over {@link VIZ_FLOOR_DB}..0, clamped; NaN is silence. */
export function dbToUnit(db: number): number {
  if (Number.isNaN(db)) return 0;
  return clamp01((db - VIZ_FLOOR_DB) / -VIZ_FLOOR_DB);
}

/** The inverse of {@link dbToUnit} inside 0..1. */
export function unitToDb(unit: number): number {
  return VIZ_FLOOR_DB + clamp01(unit) * -VIZ_FLOOR_DB;
}

/** Position on the natural-log frequency axis. */
const ln = Math.log;

/**
 * Re-bins `source` (log-spaced, low first, each 0..1) onto the contract's 32
 * log-spaced bands from 30 Hz to 16 kHz.
 *
 * A target takes the loudest source band that overlaps it enough: each source
 * value is already the loudest bin of its range, so the max over several is
 * the loudest bin of their union. A target entirely outside the source's range
 * (only possible if `sourceHighHz` is below 16 kHz) is silent.
 */
export function rebinBands(source: readonly number[], sourceHighHz: number = SOURCE_HIGH_HZ): number[] {
  const out = new Array<number>(VIZ_BAND_COUNT).fill(0);
  const n = source.length;
  if (n === 0 || !(sourceHighHz > SOURCE_LOW_HZ)) return out;

  const srcLo = ln(SOURCE_LOW_HZ);
  const srcW = (ln(sourceHighHz) - srcLo) / n;
  const tgtLo = ln(VIZ_MIN_HZ);
  const tgtW = (ln(VIZ_MAX_HZ) - tgtLo) / VIZ_BAND_COUNT;

  for (let i = 0; i < VIZ_BAND_COUNT; i++) {
    const from = tgtLo + i * tgtW;
    const to = from + tgtW;
    let best = -1; // loudest qualifying band
    let widest = 0; // largest overlap seen
    let widestValue = 0;
    for (let k = 0; k < n; k++) {
      const a = srcLo + k * srcW;
      const overlap = Math.min(to, a + srcW) - Math.max(from, a);
      if (overlap <= 0) continue;
      const value = clamp01(source[k] ?? 0);
      if (overlap >= MIN_OVERLAP * srcW && value > best) best = value;
      if (overlap > widest) {
        widest = overlap;
        widestValue = value;
      }
    }
    out[i] = round2(best >= 0 ? best : widestValue);
  }
  return out;
}

/**
 * Overall loudness 0..1: the mean power of the bands, as dBFS, mapped like the
 * bands. The mean, not the sum — each band is already a per-bin level, so
 * summing 64 of them would read ~18 dB too loud.
 */
export function vizLevel(source: readonly number[]): number {
  if (source.length === 0) return 0;
  let power = 0;
  for (const v of source) power += 10 ** (unitToDb(v) / 10);
  return round2(dbToUnit(10 * Math.log10(power / source.length)));
}

/** Source bands whose range lies below {@link BASS_MAX_HZ}. */
export function bassBandCount(n: number = SOURCE_BANDS, sourceHighHz: number = SOURCE_HIGH_HZ): number {
  const share = ln(BASS_MAX_HZ / SOURCE_LOW_HZ) / ln(sourceHighHz / SOURCE_LOW_HZ);
  return Math.max(1, Math.min(n, Math.round(n * share)));
}

/** Loudness of the bass in dBFS: the mean power of the lowest bands. */
export function bassDb(source: readonly number[], bands: number = bassBandCount(source.length)): number {
  const k = Math.min(bands, source.length);
  if (k === 0) return VIZ_FLOOR_DB;
  let power = 0;
  for (let i = 0; i < k; i++) power += 10 ** (unitToDb(source[i] ?? 0) / 10);
  return 10 * Math.log10(power / k);
}
