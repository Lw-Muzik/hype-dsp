import { useCallback, useRef } from "react";
import { cn } from "@/lib/cn";

interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Accessible name, e.g. "Master volume". */
  label: string;
  disabled?: boolean;
  onChange: (value: number) => void;
  /** Fires once when an interaction *ends* — pointer release, or a keyboard
   *  step — carrying the final value. `onChange` streams every intermediate
   *  value during a drag; `onCommit` is for consumers whose real work is
   *  expensive to repeat (a seek that re-opens a network stream) and should run
   *  only when the user settles. Optional; controls that want live continuous
   *  updates (volume, EQ) simply don't pass it. */
  onCommit?: (value: number) => void;
  /** Render the live value for screen readers / tooltips. */
  formatValue?: (value: number) => string;
  className?: string;
}

const clamp = (n: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, n));

/**
 * A keyboard- and pointer-operable horizontal slider with proper
 * `role="slider"` ARIA semantics. Used for master volume and, later, every
 * continuous control in the app.
 */
export function Slider({
  value,
  min,
  max,
  step = (max - min) / 100,
  label,
  disabled = false,
  onChange,
  onCommit,
  formatValue,
  className,
}: SliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  // The last value this control produced, so a pointer-release can commit it
  // without recomputing from a stale event.
  const lastRef = useRef(value);

  const fraction = clamp((value - min) / (max - min), 0, 1);

  const setFromClientX = useCallback(
    (clientX: number) => {
      const track = trackRef.current;
      if (!track) return;
      const rect = track.getBoundingClientRect();
      if (rect.width === 0) return;
      const f = clamp((clientX - rect.left) / rect.width, 0, 1);
      const raw = min + f * (max - min);
      const snapped = Math.round(raw / step) * step;
      const v = clamp(snapped, min, max);
      lastRef.current = v;
      onChange(v);
    },
    [min, max, step, onChange],
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (disabled) return;
      e.preventDefault();
      (e.target as Element).setPointerCapture?.(e.pointerId);
      setFromClientX(e.clientX);
    },
    [disabled, setFromClientX],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (disabled || e.buttons === 0) return;
      setFromClientX(e.clientX);
    },
    [disabled, setFromClientX],
  );

  // Commit at the end of a drag. Both pointerup and lostpointercapture, because
  // the capture can be yanked (a system gesture, focus loss) without a clean up.
  const onPointerEnd = useCallback(() => {
    if (disabled) return;
    onCommit?.(lastRef.current);
  }, [disabled, onCommit]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (disabled) return;
      const big = (max - min) / 10;
      let next: number | null = null;
      switch (e.key) {
        case "ArrowRight":
        case "ArrowUp":
          next = value + step;
          break;
        case "ArrowLeft":
        case "ArrowDown":
          next = value - step;
          break;
        case "PageUp":
          next = value + big;
          break;
        case "PageDown":
          next = value - big;
          break;
        case "Home":
          next = min;
          break;
        case "End":
          next = max;
          break;
        default:
          return;
      }
      e.preventDefault();
      const v = clamp(next, min, max);
      lastRef.current = v;
      onChange(v);
      // A key press is a discrete, settled change — commit it at once (one seek
      // per press, never a storm).
      onCommit?.(v);
    },
    [disabled, value, step, min, max, onChange, onCommit],
  );

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-valuetext={formatValue?.(value)}
      aria-disabled={disabled || undefined}
      tabIndex={disabled ? -1 : 0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onLostPointerCapture={onPointerEnd}
      onKeyDown={onKeyDown}
      className={cn(
        "group relative flex h-5 cursor-pointer items-center",
        // Without an explicit width the track collapses to ~0px, which both
        // hides the control and makes `getBoundingClientRect().width === 0`
        // (so pointer drags are ignored). Default to filling the row.
        !className && "flex-1",
        disabled && "cursor-not-allowed opacity-50",
        className,
      )}
    >
      {/* Track */}
      <div className="h-1.5 w-full rounded-full bg-border-strong">
        {/* Fill */}
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-75"
          style={{ width: `${fraction * 100}%` }}
        />
      </div>
      {/* Thumb */}
      <div
        className={cn(
          "pointer-events-none absolute size-3.5 -translate-x-1/2 rounded-full",
          "bg-text shadow-sm ring-2 ring-surface transition-transform",
          "group-hover:scale-110 group-focus-visible:scale-110",
        )}
        style={{ left: `${fraction * 100}%` }}
      />
    </div>
  );
}
