/**
 * Keeping a muted `<video>` beside DSP-chain audio.
 *
 * Two clocks exist and only one is the truth. The engine owns playback — it
 * decodes, it applies the chain, it is what you hear — so the picture follows
 * it, never the reverse. The video element is never allowed to influence audio
 * in any way; the worst a broken video may do is look wrong.
 *
 * They drift regardless: separate decoders, separate buffering, and a video
 * whose bytes arrive over a link that may be slower than the audio's. So the
 * position is compared each progress tick and the video is nudged only when the
 * gap is big enough to see. Correcting continuously would mean seeking on every
 * tick, which stutters far worse than the drift it fixes.
 *
 * The decision is a pure function so it can be tested without a DOM, a network,
 * or an audio engine — the parts that make this hard to reason about are exactly
 * the parts worth isolating from it.
 */

/** How far the picture may drift before it's worth a correction.
 *
 *  Under this, a viewer can't tell — and a seek costs a decode flush and a
 *  visible stutter, so correcting it would be the more noticeable of the two
 *  faults. Music video is also forgiving in a way film isn't: nobody lip-reads
 *  a music video, and the beat you're hearing is the engine's regardless. */
export const DRIFT_TOLERANCE_SECS = 0.25;

/** A video that has drifted this far isn't drifting, it's lost — a stall, a
 *  buffer starve, or a seek the element never saw. Correct it the same way; the
 *  constant exists to name the case, not to branch on it. */
export const DRIFT_LOST_SECS = 5;

export interface SyncInputs {
  /** The engine's position — the truth. */
  enginePos: number;
  /** The element's own clock. */
  videoPos: number;
  /** Whether the engine considers itself paused. */
  paused: boolean;
  /** Whether the element is currently paused. */
  videoPaused: boolean;
  /** Whether the element has enough data to play at all. Correcting a video
   *  that hasn't buffered yet just fights its loader. */
  ready: boolean;
}

export interface SyncAction {
  /** Seek the element here, or null to leave its clock alone. */
  seekTo: number | null;
  /** Play/pause the element, or null to leave it as it is. */
  setPaused: boolean | null;
}

/**
 * What the picture should do to match the sound.
 *
 * Ordering matters: transport before position. Seeking a video that is about to
 * be paused is wasted work, and a video left playing while the engine is paused
 * runs away — the drift then grows without bound instead of being corrected.
 */
export function syncAction(i: SyncInputs): SyncAction {
  const setPaused = i.videoPaused !== i.paused ? i.paused : null;

  // Nothing decoded yet: its clock is meaningless and seeking only interrupts
  // the buffering that would make it meaningful.
  if (!i.ready) return { seekTo: null, setPaused };

  const drift = Math.abs(i.videoPos - i.enginePos);
  return {
    seekTo: drift > DRIFT_TOLERANCE_SECS ? i.enginePos : null,
    setPaused,
  };
}
