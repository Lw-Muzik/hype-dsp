import { Pause, Play, SkipBack, SkipForward } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { Slider } from "@/components/Slider";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

const iconBtn =
  "flex size-9 items-center justify-center rounded-full text-text-muted transition-colors hover:bg-surface-overlay hover:text-text disabled:pointer-events-none disabled:opacity-40";

/** Playback transport: now-playing, prev/play-pause/next, and a seek bar. */
export function TransportBar() {
  const playing = useEngineStore((s) => s.playing);
  const paused = useEngineStore((s) => s.paused);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const positionSecs = useEngineStore((s) => s.positionSecs);
  const durationSecs = useEngineStore((s) => s.durationSecs);
  const queue = useEngineStore((s) => s.queue);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  const togglePause = useEngineStore((s) => s.togglePause);
  const next = useEngineStore((s) => s.next);
  const prev = useEngineStore((s) => s.prev);
  const seek = useEngineStore((s) => s.seek);

  const hasPrev = queueIndex > 0;
  const hasNext = queueIndex >= 0 && queueIndex + 1 < queue.length;
  const duration = durationSecs ?? 0;
  const showPause = playing && !paused;

  return (
    <div className="flex items-center gap-4 rounded-card border border-border bg-surface-raised px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">
          {nowPlaying ?? "Nothing playing"}
        </p>
        <div className="mt-1.5 flex items-center gap-2">
          <span className="w-9 text-right text-[11px] tabular-nums text-text-faint">
            {formatTime(positionSecs)}
          </span>
          <Slider
            label="Seek"
            min={0}
            max={Math.max(duration, 0.1)}
            step={0.1}
            value={Math.min(positionSecs, duration > 0 ? duration : positionSecs)}
            onChange={seek}
            disabled={!nowPlaying || duration <= 0}
            formatValue={(v) => formatTime(v)}
            className="flex-1"
          />
          <span className="w-9 text-[11px] tabular-nums text-text-faint">
            {formatTime(durationSecs)}
          </span>
        </div>
      </div>

      <div className="flex items-center gap-1">
        <button
          type="button"
          aria-label="Previous track"
          onClick={prev}
          disabled={!hasPrev}
          className={iconBtn}
        >
          <SkipBack className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label={showPause ? "Pause" : "Play"}
          onClick={togglePause}
          disabled={!nowPlaying}
          className={cn(
            "flex size-10 items-center justify-center rounded-full bg-accent text-on-accent transition-colors hover:bg-accent-strong",
            "disabled:pointer-events-none disabled:opacity-40",
          )}
        >
          {showPause ? (
            <Pause className="size-5" aria-hidden="true" />
          ) : (
            <Play className="size-5" aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          aria-label="Next track"
          onClick={next}
          disabled={!hasNext}
          className={iconBtn}
        >
          <SkipForward className="size-4" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
