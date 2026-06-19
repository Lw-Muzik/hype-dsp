/**
 * Energy-onset beat detection.
 *
 * A simple, robust adaptive detector: track a slow running average of the
 * channel level and fire a "pulse" whenever the instantaneous level jumps well
 * above that average (an onset / transient — kicks, snares, plucks). The pulse
 * then decays smoothly, so the UI reads as pulsing on the beat without needing
 * real tempo analysis. Frame-rate independent via the `dt` term.
 */

/** Per-channel detector state. */
export interface BeatState {
  /** Slow running average level — the adaptive onset threshold baseline. */
  avg: number;
  /** Current pulse envelope (0..1): spikes on an onset, then decays. */
  pulse: number;
}

export const initialBeat = (): BeatState => ({ avg: 0, pulse: 0 });

/** Instantaneous level must exceed `avg * ONSET_RATIO` to count as an onset. */
const ONSET_RATIO = 1.45;
/** Ignore onsets below this absolute level (near-silence / noise floor). */
const ONSET_FLOOR = 0.06;
/** Pulse decay time constant in seconds (smaller = snappier). */
const PULSE_TAU = 0.13;
/** Running-average smoothing per ~60fps frame. */
const AVG_ALPHA = 0.12;

/**
 * Advance one channel's beat envelope by a single frame.
 *
 * @param s     previous state
 * @param level current channel level, 0..1
 * @param dt    seconds since the previous frame (keeps decay rate-independent)
 * @returns the next state; `pulse` is the value to render
 */
export function stepBeat(s: BeatState, level: number, dt: number): BeatState {
  const v = Number.isFinite(level) ? Math.max(0, level) : 0;
  const onset = v > ONSET_FLOOR && v > s.avg * ONSET_RATIO;
  const decayed = s.pulse * Math.exp(-Math.max(0, dt) / PULSE_TAU);
  // An onset snaps the pulse up to the onset strength; otherwise it decays.
  const pulse = onset ? Math.min(1, Math.max(decayed, v)) : decayed;
  const avg = s.avg + (v - s.avg) * AVG_ALPHA;
  return { avg, pulse };
}
