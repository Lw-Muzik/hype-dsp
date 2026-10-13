/**
 * Bass-hit detection from the engine's spectrum: a kick, an 808, a log drum —
 * whatever lands in the low end with an attack. A port of the phone's
 * `BassBeat` (`hype/lib/visualizers/bass_beat.dart`) to a frame rate of ~30 Hz.
 *
 * # Why not `lib/beat.ts`
 *
 * `stepBeat` watches the whole mix's level and fires at 1.45x its running
 * average. A mastered song barely moves its full-band level between kicks, so
 * on the phone that detector answered only 21-53% of the bass hits (and pulsed
 * on snares and vocal stabs). This listens to the band below ~150 Hz only, in
 * decibels, where "a hit" is a rise of a few dB whatever the loudness.
 *
 * # How a hit is told from everything else
 *
 * - **It rises sharply** — at least {@link FAST_RISE_DB} over the lowest of the
 *   last {@link FAST_FRAMES} frames.
 * - **It clears where the bass was** — at least {@link RISE_DB} over a baseline
 *   trailing the level by ~{@link BASE_TAU} s (a kick's tail cancelling the
 *   bassline recovers fast but only back to the baseline).
 * - **It is loud for this song** — within {@link GATE_DB} of the loudest bass
 *   of the last few seconds, and above an absolute floor.
 * - **It is not the same hit** — {@link REFRACTORY} s apart; once a hit fires
 *   the baseline starts just under its peak so its own tail can't re-arm.
 *
 * It fires the moment the threshold is crossed and is *topped up* while the
 * attack keeps rising (up to {@link ATTACK_WINDOW} s): the picture can move
 * with the sound instead of ~30 ms after it.
 */

/** Frames the "sharp rise" looks back over. The engine's FFT window (43 ms)
 *  already smears an attack across two frames, so two, not the phone's ~17 ms. */
export const FAST_FRAMES = 2;
export const FAST_RISE_DB = 3.0;
export const RISE_DB = 3.5;
export const GATE_DB = 22.0;
export const FLOOR_DB = -58.0;
/** Strength spans this many dB below the recent loudest bass. */
export const RANGE_DB = 14.0;
/** Below this a hit would barely move anything: noise, not a beat. */
export const MIN_STRENGTH = 0.15;
export const REFRACTORY = 0.11;
export const ATTACK_WINDOW = 0.04;
export const BASE_TAU = 0.09;
export const SETTLE_DB = 3.0;
/** How fast "the loudest bass lately" forgets, so a quiet verse after a loud
 *  drop still gets full-strength hits within a few seconds. */
export const PEAK_RELEASE_DB_PER_S = 3.0;
/** Frames since a reset before a hit may fire (the baseline starts from nothing). */
const WARMUP_FRAMES = 4;

export interface BassHit {
  /** 0..1 against the song's recent loudest bass. */
  strength: number;
  /** What this report adds: the whole strength when the hit starts, the increase after. */
  gain: number;
  /** Whether this starts a hit (rather than topping one up). */
  onset: boolean;
}

export class BassBeat {
  private recent: number[] = [];
  private base: number | null = null;
  private peak = -120;
  private clock = 0;
  private lastOnset = -1e9;
  private seen = 0;
  private openAt: number | null = null;
  private openBest = 0;
  private openReported = 0;

  /** Feed one frame's bass level (dBFS) `dt` seconds after the previous one;
   *  returns the hit it holds, if any. */
  step(db: number, dt: number): BassHit | null {
    const level = Number.isFinite(db) ? db : -120;
    const hop = Number.isFinite(dt) ? Math.max(0, dt) : 0;
    this.seen += 1;

    let lowest = level;
    if (this.recent.length > 0) lowest = Math.min(...this.recent);
    const fastRise = this.recent.length > 0 ? level - lowest : 0;
    this.recent.push(level);
    if (this.recent.length > FAST_FRAMES) this.recent.shift();

    this.clock += hop;
    if (this.base === null) this.base = level;
    this.peak = level > this.peak ? level : Math.max(level, this.peak - PEAK_RELEASE_DB_PER_S * hop);

    let out: BassHit | null = null;
    if (this.openAt !== null) {
      if (level > this.openBest && this.clock - this.openAt <= ATTACK_WINDOW) {
        this.openBest = level;
        const s = this.strengthOf(level);
        if (s > this.openReported) {
          out = { strength: s, gain: s - this.openReported, onset: false };
          this.openReported = s;
        }
      } else {
        this.openAt = null;
        this.base = Math.max(this.base, this.openBest - SETTLE_DB);
      }
    }
    if (
      this.openAt === null &&
      out === null &&
      level - this.base > RISE_DB &&
      fastRise >= FAST_RISE_DB &&
      level > this.peak - GATE_DB &&
      level > FLOOR_DB &&
      this.seen > WARMUP_FRAMES &&
      this.clock - this.lastOnset > REFRACTORY
    ) {
      const s = this.strengthOf(level);
      if (s >= MIN_STRENGTH) {
        this.openAt = this.clock;
        this.openBest = level;
        this.openReported = s;
        this.lastOnset = this.clock;
        out = { strength: s, gain: s, onset: true };
      }
    }
    if (this.openAt === null) {
      this.base += (level - this.base) * (1 - Math.exp(-hop / BASE_TAU));
    }
    return out;
  }

  /** Forget the song: a new track or a restart begins from nothing. */
  reset(): void {
    this.recent = [];
    this.base = null;
    this.peak = -120;
    this.clock = 0;
    this.lastOnset = -1e9;
    this.seen = 0;
    this.openAt = null;
    this.openBest = 0;
    this.openReported = 0;
  }

  private strengthOf(db: number): number {
    return Math.min(1, Math.max(0, (db - (this.peak - RANGE_DB)) / RANGE_DB));
  }
}
