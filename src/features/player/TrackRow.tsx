import type { ReactNode } from "react";
import { Cloud, Play, Smartphone, SquarePlay } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Artwork } from "@/features/player/Artwork";
import type { ArtSource } from "@/lib/useTrackArtwork";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

/** Which source a row came from, for the corner badge. "local" shows none. */
export type RowSource = "local" | "phone" | "cloud" | "ytmusic";

const BADGE: Record<Exclude<RowSource, "local">, { icon: LucideIcon; title: string }> = {
  phone: { icon: Smartphone, title: "From your phone" },
  cloud: { icon: Cloud, title: "From the cloud" },
  ytmusic: { icon: SquarePlay, title: "From YouTube Music" },
};

/**
 * One song row: rank, cover art (real embedded/phone art, lazily loaded),
 * title + artist, a small badge for non-local sources, duration, and an
 * optional trailing action. Clicking the row plays it. Source-agnostic so the
 * unified library can list local, phone, cloud, and YouTube Music tracks
 * together.
 */
export function TrackRow({
  rank,
  title,
  artist,
  durationSecs,
  art,
  seed,
  source = "local",
  unavailable = false,
  playing,
  onPlay,
  trailing,
}: {
  rank: number;
  title: string;
  artist: string | null;
  durationSecs: number | null;
  /** Where to resolve the cover art from (omit for gradient-only). */
  art?: ArtSource | null;
  /** Gradient-fallback seed (usually album or title). */
  seed: string;
  source?: RowSource;
  /** Listed but not playable (a removed / region-blocked YT Music track). */
  unavailable?: boolean;
  playing: boolean;
  onPlay: () => void;
  trailing?: ReactNode;
}) {
  const badge = source === "local" ? null : BADGE[source];
  return (
    <div
      onClick={unavailable ? undefined : onPlay}
      aria-disabled={unavailable || undefined}
      title={unavailable ? "Not available on YouTube Music" : undefined}
      className={cn(
        "group flex h-full items-center gap-3 rounded-control px-2 transition-colors",
        unavailable
          ? "cursor-not-allowed"
          : "cursor-pointer hover:bg-surface-overlay",
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
      <div className={cn("relative", unavailable && "opacity-40 grayscale")}>
        <Artwork art={art} seed={seed} label={title} rounded="rounded-md" className="size-11" />
        {!unavailable && (
          <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
            <Play className="size-4 text-white" aria-hidden="true" />
          </span>
        )}
        {badge && (
          <span
            className="absolute -bottom-1 -right-1 grid size-4 place-items-center rounded-full bg-surface-raised text-accent ring-1 ring-border"
            title={badge.title}
          >
            <badge.icon className="size-2.5" aria-hidden="true" />
          </span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p
          className={cn(
            "truncate text-sm font-medium",
            playing && "text-accent-strong",
            unavailable && "text-text-faint line-through",
          )}
        >
          {title}
        </p>
        <p className="truncate text-xs text-text-muted">
          {unavailable ? "Unavailable" : (artist ?? "Unknown artist")}
        </p>
      </div>
      <span className="w-14 shrink-0 text-right text-xs tabular-nums text-text-muted">
        {unavailable ? "—" : formatTime(durationSecs)}
      </span>
      {trailing && (
        <div onClick={(e) => e.stopPropagation()} className="shrink-0">
          {trailing}
        </div>
      )}
    </div>
  );
}
