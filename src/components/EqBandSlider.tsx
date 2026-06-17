import { useCallback, useRef } from "react";
import { formatDb } from "@/lib/format";

interface EqBandSliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Accessible name, e.g. "1k". */
  label: string;
  onChange: (value: number) => void;
}

const clamp = (n: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, n));

/** A vertical, keyboard- and pointer-operable EQ band fader (0 dB centered). */
export function EqBandSlider({
  value,
  min,
  max,
  step = 0.5,
  label,
  onChange,
}: EqBandSliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  // 0 (bottom) .. 1 (top)
  const frac = clamp((value - min) / (max - min), 0, 1);
  const zeroFrac = clamp((0 - min) / (max - min), 0, 1);

  const setFromY = useCallback(
    (clientY: number) => {
      const el = trackRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      if (r.height === 0) return;
      const f = clamp(1 - (clientY - r.top) / r.height, 0, 1);
      const raw = min + f * (max - min);
      onChange(clamp(Math.round(raw / step) * step, min, max));
    },
    [min, max, step, onChange],
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      (e.target as Element).setPointerCapture?.(e.pointerId);
      setFromY(e.clientY);
    },
    [setFromY],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.buttons === 0) return;
      setFromY(e.clientY);
    },
    [setFromY],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      let next: number | null = null;
      switch (e.key) {
        case "ArrowUp":
          next = value + step;
          break;
        case "ArrowDown":
          next = value - step;
          break;
        case "Home":
          next = max;
          break;
        case "End":
          next = min;
          break;
        default:
          return;
      }
      e.preventDefault();
      onChange(clamp(next, min, max));
    },
    [value, step, min, max, onChange],
  );

  const fillTop = (1 - Math.max(frac, zeroFrac)) * 100;
  const fillHeight = Math.abs(frac - zeroFrac) * 100;

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label={`${label} hertz band`}
      aria-orientation="vertical"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-valuetext={`${formatDb(value)} decibels`}
      tabIndex={0}
      title={`${label}: ${formatDb(value)} dB`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onKeyDown={onKeyDown}
      className="relative w-4 flex-1 cursor-pointer touch-none"
    >
      <div className="absolute left-1/2 top-0 h-full w-1 -translate-x-1/2 rounded-full bg-border-strong" />
      <div
        className="absolute left-1/2 w-2 -translate-x-1/2 border-t border-border"
        style={{ top: `${(1 - zeroFrac) * 100}%` }}
        aria-hidden="true"
      />
      <div
        className="absolute left-1/2 w-1 -translate-x-1/2 rounded-full bg-accent"
        style={{ top: `${fillTop}%`, height: `${fillHeight}%` }}
        aria-hidden="true"
      />
      <div
        className="absolute left-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-text ring-2 ring-surface transition-transform hover:scale-110"
        style={{ top: `${(1 - frac) * 100}%` }}
        aria-hidden="true"
      />
    </div>
  );
}
