import { useEffect, useState } from "react";
import { useEngineStore } from "@/stores/engine";
import { initialBeat, stepBeat, type BeatState } from "@/lib/beat";
import type { MeterFrame } from "@/lib/types";
import { cn } from "@/lib/cn";

const channelWidth = (peak: number): number =>
  Math.min(100, Math.max(0, peak * 100));

/** Linear interpolate an integer channel between `a` and `b` by `t` (0..1). */
const mix = (a: number, b: number, t: number): number =>
  Math.round(a + (b - a) * Math.min(1, Math.max(0, t)));

// Label colour endpoints: muted (idle / between beats) → accent gold (on beat).
const IDLE_RGB = [120, 124, 138] as const;
const ACCENT_RGB = [245, 180, 15] as const;

interface MeterView {
  peak: [number, number];
  pulse: [number, number];
  live: boolean;
}

/**
 * Compact stereo output meter for the top bar. It reads live meter frames
 * straight off the engine store on its own animation frame — so the top bar
 * doesn't re-render at 60fps — and pulses the L/R indicators on detected beats
 * (energy-onset detection per channel). Idle when nothing is playing.
 */
export function LevelMeter({ className }: { className?: string }) {
  const [view, setView] = useState<MeterView>({
    peak: [0, 0],
    pulse: [0, 0],
    live: false,
  });

  useEffect(() => {
    const beat: [BeatState, BeatState] = [initialBeat(), initialBeat()];
    let raf = 0;
    let prev = performance.now();
    // Once idle settles to zero, stop pushing state (the rAF keeps spinning, but
    // cheaply) until signal returns — no 60fps re-renders while nothing plays.
    let settled = false;
    // Meter frames land at ~30fps while this rAF runs at display rate
    // (60–120Hz): remember the last frame seen so ticks with no new data skip
    // the React state write instead of re-rendering for identical values.
    let lastMeters: MeterFrame | null = null;
    const tick = (now: number) => {
      const dt = Math.min(0.1, (now - prev) / 1000);
      prev = now;
      const s = useEngineStore.getState();
      const metersChanged = s.meters !== lastMeters;
      lastMeters = s.meters;
      const live = s.metersLive && s.playing && !s.paused;
      const peak: [number, number] = [
        live ? (s.meters.peak[0] ?? 0) : 0,
        live ? (s.meters.peak[1] ?? 0) : 0,
      ];
      beat[0] = stepBeat(beat[0], peak[0], dt);
      beat[1] = stepBeat(beat[1], peak[1], dt);
      const idle =
        !live &&
        peak[0] === 0 &&
        peak[1] === 0 &&
        beat[0].pulse < 0.005 &&
        beat[1].pulse < 0.005;
      if (idle) {
        if (!settled) {
          settled = true;
          setView({ peak: [0, 0], pulse: [0, 0], live: false });
        }
      } else {
        settled = false;
        // Write only when a fresh meter frame arrived. When frames stop while
        // a pulse is still glowing (the brief tail after pause/stop), keep
        // writing so the decay animates to zero before the idle branch settles.
        const pulseFading =
          !live && (beat[0].pulse >= 0.005 || beat[1].pulse >= 0.005);
        if (metersChanged || pulseFading) {
          setView({ peak, pulse: [beat[0].pulse, beat[1].pulse], live });
        }
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  const channels = ["L", "R"] as const;
  const valueNow = Math.round(Math.max(view.peak[0], view.peak[1]) * 100);

  return (
    <div
      role="meter"
      aria-label={view.live ? "Output level" : "Output level (idle)"}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={valueNow}
      className={cn("flex flex-col justify-center gap-1", className)}
    >
      {channels.map((ch, i) => {
        const pulse = view.live ? (view.pulse[i] ?? 0) : 0;
        const r = mix(IDLE_RGB[0], ACCENT_RGB[0], pulse);
        const g = mix(IDLE_RGB[1], ACCENT_RGB[1], pulse);
        const b = mix(IDLE_RGB[2], ACCENT_RGB[2], pulse);
        return (
          <div key={ch} className="flex items-center gap-1.5">
            <span
              className="w-2 text-[10px] font-bold will-change-transform"
              style={{
                color: `rgb(${r}, ${g}, ${b})`,
                transform: `scale(${1 + pulse * 0.45})`,
                textShadow:
                  pulse > 0.04
                    ? `0 0 ${pulse * 7}px rgba(245, 180, 15, ${pulse * 0.85})`
                    : "none",
              }}
            >
              {ch}
            </span>
            <div
              className={cn(
                "h-1.5 w-16 overflow-hidden rounded-full bg-border",
                !view.live && "opacity-50",
              )}
            >
              <div
                className="h-full rounded-full bg-accent"
                style={{
                  width: `${channelWidth(view.peak[i] ?? 0)}%`,
                  filter: `brightness(${1 + pulse * 0.8})`,
                }}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
