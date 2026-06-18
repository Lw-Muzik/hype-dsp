import { Pause, Play, SkipBack, SkipForward, Volume2 } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { Slider } from "@/components/Slider";
import { formatTime } from "@/lib/format";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";

const iconBtn =
  "flex size-8 items-center justify-center rounded-full text-text-muted transition-colors hover:text-text disabled:pointer-events-none disabled:opacity-30";

/** The current track's cover: embedded art, else a gradient with initials. */
function Cover({ cover, seed }: { cover: string | null; seed: string }) {
  if (cover) {
    return (
      <img
        src={cover}
        alt=""
        className="size-12 shrink-0 rounded-lg object-cover shadow-sm"
        aria-hidden="true"
      />
    );
  }
  return (
    <div
      className="grid size-12 shrink-0 place-items-center rounded-lg text-sm font-semibold text-white/90 shadow-sm"
      style={{ background: coverGradient(seed) }}
      aria-hidden="true"
    >
      <span className="opacity-80">{coverInitials(seed)}</span>
    </div>
  );
}

/**
 * Persistent now-playing bar docked at the bottom of every view: cover art +
 * title/artist, transport with a circular accent play button, a seek bar, and
 * a master volume control. Driven entirely by the engine store, so it reflects
 * whatever is playing — local, cloud, or phone.
 */
export function NowPlayingBar() {
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const playing = useEngineStore((s) => s.playing);
  const paused = useEngineStore((s) => s.paused);
  const positionSecs = useEngineStore((s) => s.positionSecs);
  const durationSecs = useEngineStore((s) => s.durationSecs);
  const queue = useEngineStore((s) => s.queue);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  const togglePause = useEngineStore((s) => s.togglePause);
  const next = useEngineStore((s) => s.next);
  const prev = useEngineStore((s) => s.prev);
  const seek = useEngineStore((s) => s.seek);
  const masterVolume = useEngineStore((s) => s.state.masterVolume);
  const setMasterVolume = useEngineStore((s) => s.setMasterVolume);

  const active = meta !== null;
  const hasPrev = queueIndex > 0;
  const hasNext = queueIndex >= 0 && queueIndex + 1 < queue.length;
  const duration = durationSecs ?? 0;
  const showPause = playing && !paused;
  const title = meta?.title ?? "Nothing playing";
  const subtitle = meta?.artist ?? meta?.album ?? null;

  return (
    <footer className="flex h-[76px] shrink-0 items-center gap-4 border-t border-border bg-surface-raised px-4">
      {/* Track */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <Cover cover={meta?.cover ?? null} seed={meta?.title ?? "—"} />
        <div className="min-w-0">
          <p
            className={cn(
              "truncate text-sm font-medium",
              !active && "text-text-faint",
            )}
          >
            {title}
          </p>
          {subtitle && (
            <p className="truncate text-xs text-text-muted">{subtitle}</p>
          )}
        </div>
      </div>

      {/* Transport + seek */}
      <div className="flex max-w-xl flex-[1.6] flex-col items-center gap-1.5">
        <div className="flex items-center gap-2">
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
            disabled={!active}
            className={cn(
              "flex size-10 items-center justify-center rounded-full bg-accent text-surface shadow transition-transform hover:scale-105 active:scale-100",
              "disabled:pointer-events-none disabled:opacity-30",
            )}
          >
            {showPause ? (
              <Pause className="size-5 fill-current" aria-hidden="true" />
            ) : (
              <Play className="size-5 fill-current" aria-hidden="true" />
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
        <div className="flex w-full items-center gap-2">
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
            disabled={!active || duration <= 0}
            formatValue={(v) => formatTime(v)}
            className="flex-1"
          />
          <span className="w-9 text-[11px] tabular-nums text-text-faint">
            {duration > 0 ? formatTime(durationSecs) : "—:—"}
          </span>
        </div>
      </div>

      {/* Volume */}
      <div className="flex flex-1 items-center justify-end gap-2">
        <Volume2 className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
        <Slider
          label="Volume"
          min={0}
          max={1}
          step={0.01}
          value={masterVolume}
          onChange={setMasterVolume}
          formatValue={(v) => `${Math.round(v * 100)}%`}
          className="w-28"
        />
      </div>
    </footer>
  );
}
