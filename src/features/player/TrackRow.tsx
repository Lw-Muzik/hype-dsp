import type { ReactNode } from "react";
import { Play } from "lucide-react";
import { Artwork } from "@/features/player/Artwork";
import { formatTime } from "@/lib/format";
import type { LibraryTrack } from "@/lib/types";
import { cn } from "@/lib/cn";

/**
 * One song in the list: rank, cover art (real ID3 art, lazily loaded), title +
 * artist, duration, and an optional trailing action (add to / remove from a
 * playlist). Clicking the row plays it.
 */
export function TrackRow({
  track,
  rank,
  playing,
  onPlay,
  trailing,
}: {
  track: LibraryTrack;
  rank: number;
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
        <Artwork
          path={track.path}
          seed={track.album?.trim() || track.title}
          label={track.title}
          rounded="rounded-md"
          className="size-11"
        />
        <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
          <Play className="size-4 text-white" aria-hidden="true" />
        </span>
      </div>
      <div className="min-w-0 flex-1">
        <p
          className={cn(
            "truncate text-sm font-medium",
            playing && "text-accent-strong",
          )}
        >
          {track.title}
        </p>
        <p className="truncate text-xs text-text-muted">
          {track.artist ?? "Unknown artist"}
        </p>
      </div>
      <span className="w-14 shrink-0 text-right text-xs tabular-nums text-text-muted">
        {formatTime(track.durationSecs)}
      </span>
      {trailing && (
        <div onClick={(e) => e.stopPropagation()} className="shrink-0">
          {trailing}
        </div>
      )}
    </div>
  );
}
