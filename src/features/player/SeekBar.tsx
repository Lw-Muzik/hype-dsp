import { useState } from "react";
import { Slider } from "@/components/Slider";
import { formatTime } from "@/lib/format";

/**
 * The value the thumb should sit at: the scrub position while dragging, the
 * engine's real position otherwise, clamped to the known duration.
 *
 * Pure so the "a live drag shows where you're dragging, not where playback is"
 * rule is tested without a pointer. `scrub` is `null` whenever the user isn't
 * actively dragging.
 */
export function seekDisplayValue(
  scrub: number | null,
  position: number,
  duration: number,
): number {
  if (scrub !== null) return scrub;
  return Math.min(position, duration > 0 ? duration : position);
}

/**
 * A seek bar that scrubs visually during a drag but only *seeks* on release.
 *
 * This is the whole point: a streamed source (cloud, phone, YouTube Music) seeks
 * by re-opening its network connection at the new byte offset. The shared
 * [`Slider`] streams a value on every pointer move, so wiring seek straight to
 * `onChange` fires one connection re-open per drag frame — dozens a second —
 * which starves the audio ring and stops playback. Here `onChange` only moves
 * the thumb (local state, no engine call); the real seek runs once, from
 * `onCommit`, when the user lets go. A local file wouldn't care, but the bar
 * can't tell which it is, so it treats both the safe way.
 */
export function SeekBar({
  position,
  duration,
  seekable,
  onSeek,
  className,
}: {
  position: number;
  /** Track length in seconds; 0/unknown disables seeking. */
  duration: number;
  /** Whether the active source can be scrubbed at all (streams flip to true
   *  once their length is known). */
  seekable: boolean;
  onSeek: (secs: number) => void;
  className?: string;
}) {
  const [scrub, setScrub] = useState<number | null>(null);
  const canSeek = seekable && duration > 0;

  return (
    <Slider
      label="Seek"
      min={0}
      max={Math.max(duration, 0.1)}
      step={0.1}
      value={seekDisplayValue(scrub, position, duration)}
      onChange={setScrub}
      onCommit={(v) => {
        onSeek(v);
        setScrub(null);
      }}
      disabled={!canSeek}
      formatValue={(v) => formatTime(v)}
      className={className}
    />
  );
}
