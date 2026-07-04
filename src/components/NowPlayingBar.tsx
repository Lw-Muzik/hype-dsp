import { memo, useState } from "react";
import {
  Ear,
  ListMusic,
  Loader2,
  MicVocal,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Volume2,
} from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import { clearLyricsCache } from "@/features/player/useLyrics";
import { Slider } from "@/components/Slider";
import { VisualizerButton } from "@/components/VisualizerButton";
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
 * Elapsed time + seek slider + duration. Isolated (and memoized) so the ~10Hz
 * `positionSecs` progress updates re-render only this small row instead of the
 * whole ~80-element bar.
 */
const SeekRow = memo(function SeekRow({ active }: { active: boolean }) {
  const positionSecs = useEngineStore((s) => s.positionSecs);
  const durationSecs = useEngineStore((s) => s.durationSecs);
  const buffering = useEngineStore((s) => s.buffering);
  const seekable = useEngineStore((s) => s.seekable);
  const seek = useEngineStore((s) => s.seek);

  const duration = durationSecs ?? 0;
  // A live/unknown-duration source has no scale: keep the thumb at the start
  // (never pinned to the far right) and the bar non-interactive.
  const sliderValue = duration > 0 ? Math.min(positionSecs, duration) : 0;

  return (
    <div className="flex w-full items-center gap-2">
      <span className="w-9 text-right text-[11px] tabular-nums text-text-faint">
        {buffering ? (
          <span className="animate-pulse" title="Buffering…" aria-label="Buffering">•••</span>
        ) : (
          formatTime(positionSecs)
        )}
      </span>
      <Slider
        label="Seek"
        min={0}
        max={duration > 0 ? duration : 1}
        step={0.1}
        value={sliderValue}
        onChange={seek}
        disabled={!active || !seekable || duration <= 0}
        formatValue={(v) => formatTime(v)}
        className="flex-1"
      />
      <span className="w-9 text-[11px] tabular-nums text-text-faint">
        {duration > 0 ? formatTime(durationSecs) : "—:—"}
      </span>
    </div>
  );
});

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
  // Select primitives, not the queue/order arrays: the queue array is replaced
  // by background tag enrichment and per-track patches, and this bar doesn't
  // need to re-render for those.
  const source = useEngineStore((s) =>
    s.orderPos >= 0 ? s.queue[s.order[s.orderPos] ?? -1]?.source : undefined,
  );
  const orderLen = useEngineStore((s) => s.order.length);
  const orderPos = useEngineStore((s) => s.orderPos);
  const repeat = useEngineStore((s) => s.repeat);
  const shuffle = useEngineStore((s) => s.shuffle);
  const toggleRight = useUiStore((s) => s.toggleRight);
  const rightPanel = useUiStore((s) => s.rightPanel);
  const togglePause = useEngineStore((s) => s.togglePause);
  const next = useEngineStore((s) => s.next);
  const prev = useEngineStore((s) => s.prev);
  const toggleShuffle = useEngineStore((s) => s.toggleShuffle);
  const cycleRepeat = useEngineStore((s) => s.cycleRepeat);
  const masterVolume = useEngineStore((s) => s.state.masterVolume);
  const setMasterVolume = useEngineStore((s) => s.setMasterVolume);
  const identifyNowPlaying = useEngineStore((s) => s.identifyNowPlaying);

  const [identifying, setIdentifying] = useState(false);

  const active = meta !== null;
  // Radio / cast have no list to navigate.
  const navigable = source !== "radio" && source !== "cast";
  const wrap = repeat === "all" && orderLen > 0;
  const hasPrev = navigable && orderPos >= 0 && (orderPos > 0 || wrap);
  const hasNext =
    navigable && orderPos >= 0 && (orderPos + 1 < orderLen || wrap);
  const showPause = playing && !paused;
  const title = meta?.title ?? "Nothing playing";
  const subtitle = meta?.artist ?? meta?.album ?? null;
  const repeatLabel =
    repeat === "one" ? "Repeat one" : repeat === "all" ? "Repeat all" : "Repeat off";

  // Fill any missing tags for the current track (audio fingerprint) and refresh
  // its lyrics, then reveal them — one tap to complete an under-tagged song.
  const onIdentify = async () => {
    if (!active || identifying) return;
    setIdentifying(true);
    try {
      await identifyNowPlaying();
      // Transient read — the bar deliberately doesn't subscribe to the queue.
      const { queue, queueIndex } = useEngineStore.getState();
      const current = queueIndex >= 0 ? queue[queueIndex] : undefined;
      if (current) clearLyricsCache(`${current.source}:${current.id}`);
    } finally {
      setIdentifying(false);
    }
    if (rightPanel !== "lyrics") toggleRight("lyrics");
  };

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
            aria-label="Shuffle"
            aria-pressed={shuffle}
            onClick={toggleShuffle}
            className={cn(iconBtn, shuffle && "text-accent hover:text-accent")}
          >
            <Shuffle className="size-4" aria-hidden="true" />
          </button>
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
          <button
            type="button"
            aria-label={repeatLabel}
            title={repeatLabel}
            aria-pressed={repeat !== "off"}
            onClick={cycleRepeat}
            className={cn(iconBtn, repeat !== "off" && "text-accent hover:text-accent")}
          >
            {repeat === "one" ? (
              <Repeat1 className="size-4" aria-hidden="true" />
            ) : (
              <Repeat className="size-4" aria-hidden="true" />
            )}
          </button>
        </div>
        <SeekRow active={active} />
      </div>

      {/* Identify + lyrics + queue + volume */}
      <div className="flex flex-1 items-center justify-end gap-2">
        <button
          type="button"
          aria-label="Identify song and fill missing info"
          title="Identify — fill missing tags & lyrics"
          onClick={onIdentify}
          disabled={!active || identifying}
          className={cn(iconBtn, identifying && "text-accent")}
        >
          {identifying ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <Ear className="size-4" aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          aria-label="Show lyrics"
          aria-pressed={rightPanel === "lyrics"}
          title="Lyrics"
          onClick={() => toggleRight("lyrics")}
          className={cn(iconBtn, rightPanel === "lyrics" && "text-accent hover:text-accent")}
        >
          <MicVocal className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Show queue"
          aria-pressed={rightPanel === "queue"}
          title="Queue"
          onClick={() => toggleRight("queue")}
          className={cn(iconBtn, rightPanel === "queue" && "text-accent hover:text-accent")}
        >
          <ListMusic className="size-4" aria-hidden="true" />
        </button>
        <VisualizerButton />
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
