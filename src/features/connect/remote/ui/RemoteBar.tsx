import {
  Cast,
  ListMusic,
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
import { BarCover, barIconBtn } from "@/components/BarParts";
import { Slider } from "@/components/Slider";
import { SeekBar } from "@/features/player/SeekBar";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";
import { useUiStore } from "@/stores/ui";
import type { RepeatMode } from "../../protocol";
import { sendRemote } from "../control";
import type { RemoteView } from "../session";
import { DevicePicker } from "./DevicePicker";
import { useRemotePosition } from "./useRemotePosition";
import { useRemoteVolume } from "./useRemoteVolume";

const NEXT_REPEAT: Record<RepeatMode, RepeatMode> = { off: "all", all: "one", one: "off" };
/**
 * The now-playing bar while another device is the player: that device's art,
 * title and live progress, with every control sent to it as a command. Same
 * shape as the local bar so the app doesn't shift under the user's hands.
 */
export function RemoteBar({ view }: { view: RemoteView }) {
  const rightPanel = useUiStore((s) => s.rightPanel);
  const toggleRight = useUiStore((s) => s.toggleRight);
  const positionMs = useRemotePosition(view);
  const volume = useRemoteVolume(view.volume);

  const { track, playing } = view;
  const active = track !== null;
  const title = track?.title ?? "Nothing playing";
  const subtitle = track?.artist ?? track?.album ?? null;
  const repeatLabel =
    view.repeat === "one" ? "Repeat one" : view.repeat === "all" ? "Repeat all" : "Repeat off";
  const duration = (view.durationMs ?? track?.durationMs ?? 0) / 1000;

  return (
    <footer
      aria-label={`Controlling ${view.device.name}`}
      className="flex h-[76px] shrink-0 items-center gap-4 border-t border-border bg-surface-raised px-4"
    >
      {/* Track + where it's playing */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <BarCover cover={track?.artUrl ?? null} seed={title} />
        <div className="min-w-0">
          <p className={cn("truncate font-display text-sm font-medium", !active && "text-text-faint")}>
            {title}
          </p>
          {subtitle && <p className="truncate text-xs text-text-muted">{subtitle}</p>}
          <p
            className={cn(
              "mt-0.5 flex items-center gap-1 truncate text-[11px] font-medium",
              playing ? "text-success" : "text-text-faint",
            )}
          >
            <Cast className="size-3 shrink-0" aria-hidden="true" />
            <span className="truncate">
              {playing ? "Playing on" : "Paused on"} {view.device.name}
            </span>
          </p>
        </div>
      </div>

      {/* Transport + seek */}
      <div className="flex max-w-xl flex-[1.6] flex-col items-center gap-1.5">
        <div className="flex items-center gap-2">
          <button
            type="button"
            aria-label="Shuffle"
            aria-pressed={view.shuffle}
            onClick={() => void sendRemote({ type: "shuffle", on: !view.shuffle })}
            className={cn(barIconBtn, view.shuffle && "text-accent hover:text-accent")}
          >
            <Shuffle className="size-4" aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label="Previous track"
            onClick={() => void sendRemote({ type: "prev" })}
            disabled={!active}
            className={barIconBtn}
          >
            <SkipBack className="size-4" aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label={playing ? "Pause" : "Play"}
            onClick={() => void sendRemote({ type: playing ? "pause" : "resume" })}
            disabled={!active}
            className="flex size-10 items-center justify-center rounded-full bg-accent text-on-accent shadow transition-transform hover:scale-105 active:scale-100 disabled:pointer-events-none disabled:opacity-30"
          >
            {playing ? (
              <Pause className="size-5 fill-current" aria-hidden="true" />
            ) : (
              <Play className="size-5 fill-current" aria-hidden="true" />
            )}
          </button>
          <button
            type="button"
            aria-label="Next track"
            onClick={() => void sendRemote({ type: "next" })}
            disabled={!active}
            className={barIconBtn}
          >
            <SkipForward className="size-4" aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label={repeatLabel}
            title={repeatLabel}
            aria-pressed={view.repeat !== "off"}
            onClick={() => void sendRemote({ type: "repeat", mode: NEXT_REPEAT[view.repeat] })}
            className={cn(barIconBtn, view.repeat !== "off" && "text-accent hover:text-accent")}
          >
            {view.repeat === "one" ? (
              <Repeat1 className="size-4" aria-hidden="true" />
            ) : (
              <Repeat className="size-4" aria-hidden="true" />
            )}
          </button>
        </div>
        <div className="flex w-full items-center gap-2">
          <span className="w-9 text-right text-[11px] tabular-nums text-text-faint">
            {formatTime(positionMs / 1000)}
          </span>
          <SeekBar
            position={positionMs / 1000}
            duration={duration}
            seekable={active}
            onSeek={(secs) => void sendRemote({ type: "seek", positionMs: Math.round(secs * 1000) })}
            className="flex-1"
          />
          <span className="w-9 text-[11px] tabular-nums text-text-faint">
            {duration > 0 ? formatTime(duration) : "—:—"}
          </span>
        </div>
      </div>

      {/* Lyrics + queue + devices + volume */}
      <div className="flex flex-1 items-center justify-end gap-2">
        <button
          type="button"
          aria-label="Show lyrics"
          aria-pressed={rightPanel === "lyrics"}
          title="Lyrics"
          onClick={() => toggleRight("lyrics")}
          className={cn(barIconBtn, rightPanel === "lyrics" && "text-accent hover:text-accent")}
        >
          <MicVocal className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Show queue"
          aria-pressed={rightPanel === "queue"}
          title="Queue"
          onClick={() => toggleRight("queue")}
          className={cn(barIconBtn, rightPanel === "queue" && "text-accent hover:text-accent")}
        >
          <ListMusic className="size-4" aria-hidden="true" />
        </button>
        <DevicePicker />
        <Volume2 className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
        <Slider
          label={`${view.device.name} volume`}
          min={0}
          max={1}
          step={0.01}
          value={volume.value}
          onChange={volume.onChange}
          onCommit={volume.onCommit}
          formatValue={(v) => `${Math.round(v * 100)}%`}
          className="w-28"
        />
      </div>
    </footer>
  );
}
