import type { ReactNode } from "react";
import { Cloud, Play, Smartphone } from "lucide-react";
import { Artwork } from "@/features/player/Artwork";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

/**
 * One song row: rank, cover art (real ID3 art for local tracks, lazily loaded),
 * title + artist, a small badge for phone/cloud sources, duration, and an
 * optional trailing action. Clicking the row plays it. Source-agnostic so the
 * unified library can list local, phone, and cloud tracks together.
 */
export function TrackRow({
  rank,
  title,
  artist,
  durationSecs,
  artPath,
  seed,
  source = "local",
  playing,
  onPlay,
  trailing,
}: {
  rank: number;
  title: string;
  artist: string | null;
  durationSecs: number | null;
  /** Local file path for lazy embedded art (omit for phone/cloud). */
  artPath?: string | null;
  /** Gradient-fallback seed (usually album or title). */
  seed: string;
  source?: "local" | "phone" | "cloud";
  playing: boolean;
  onPlay: () => void;
  trailing?: ReactNode;
}) {
  return (
    <div
      onClick={onPlay}
      className={cn(
        "group flex h-full cursor-pointer items-center gap-3 rounded-control px-2 transition-colors hover:bg-surface-overlay",
        playing && "bg-accent-muted/40",
      )}
    >
      <span
        className={cn(
          "w-6 text-right text-xs tabular-nums",
          playing ? "text-accent-strong" : "text-text-faint",
        )}
      >
        {String(rank).padStart(2, "0")}
      </span>
      <div className="relative">
        <Artwork path={artPath} seed={seed} label={title} rounded="rounded-md" className="size-11" />
        <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
          <Play className="size-4 text-white" aria-hidden="true" />
        </span>
        {source !== "local" && (
          <span
            className="absolute -bottom-1 -right-1 grid size-4 place-items-center rounded-full bg-surface-raised text-accent ring-1 ring-border"
            title={source === "phone" ? "From your phone" : "From the cloud"}
          >
            {source === "phone" ? (
              <Smartphone className="size-2.5" aria-hidden="true" />
            ) : (
              <Cloud className="size-2.5" aria-hidden="true" />
            )}
          </span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className={cn("truncate text-sm font-medium", playing && "text-accent-strong")}>
          {title}
        </p>
        <p className="truncate text-xs text-text-muted">{artist ?? "Unknown artist"}</p>
      </div>
      <span className="w-14 shrink-0 text-right text-xs tabular-nums text-text-muted">
        {formatTime(durationSecs)}
      </span>
      {trailing && (
        <div onClick={(e) => e.stopPropagation()} className="shrink-0">
          {trailing}
        </div>
      )}
    </div>
  );
}
