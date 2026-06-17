import { cn } from "@/lib/cn";

interface LevelMeterProps {
  /** Per-channel peak magnitude [left, right], linear 0–1+. */
  peak: [number, number];
  /** Whether real frames are flowing. When false the meter renders idle. */
  live: boolean;
  className?: string;
}

const channelWidth = (peak: number): number =>
  Math.min(100, Math.max(0, peak * 100));

/**
 * Compact stereo output meter for the top bar. Driven purely by real
 * `MeterFrame` data; in Phase 0 it sits idle (zeroed, dimmed) because no signal
 * is flowing yet — it never animates synthesized values.
 */
export function LevelMeter({ peak, live, className }: LevelMeterProps) {
  const channels = ["L", "R"] as const;
  return (
    <div
      role="meter"
      aria-label={live ? "Output level" : "Output level (idle)"}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(Math.max(peak[0], peak[1]) * 100)}
      className={cn("flex flex-col justify-center gap-1", className)}
    >
      {channels.map((ch, i) => (
        <div key={ch} className="flex items-center gap-1.5">
          <span className="w-2 text-[10px] font-medium text-text-faint">
            {ch}
          </span>
          <div
            className={cn(
              "h-1.5 w-16 overflow-hidden rounded-full bg-border",
              !live && "opacity-50",
            )}
          >
            <div
              className="h-full rounded-full bg-accent transition-[width] duration-75"
              style={{ width: `${channelWidth(peak[i] ?? 0)}%` }}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
