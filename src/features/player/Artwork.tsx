import { useTrackArtwork } from "@/lib/useTrackArtwork";
import type { ArtSource } from "@/lib/useTrackArtwork";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";

/**
 * A track/album cover: the real embedded art when present (lazily fetched,
 * source-aware, and cached), otherwise a deterministic gradient with initials.
 * Decorative — the surrounding control carries the accessible name.
 */
export function Artwork({
  art,
  seed,
  label,
  className,
  rounded = "rounded-lg",
}: {
  /** Where to resolve real cover art from (omit for gradient-only). */
  art?: ArtSource | null;
  /** Stable seed for the gradient fallback (album or title). */
  seed: string;
  /** Source for the fallback initials (usually the title). */
  label: string;
  className?: string;
  /** Tailwind rounding utility (so callers can match the surrounding shape). */
  rounded?: string;
}) {
  // A pre-resolved cover (e.g. the cloud metadata preload) wins and skips the
  // lazy fetch; otherwise resolve it source-aware.
  const direct = art?.cover ?? null;
  const fetched = useTrackArtwork(direct ? null : art);
  const cover = direct ?? fetched;
  if (cover) {
    return (
      <img
        src={cover}
        alt=""
        aria-hidden="true"
        loading="lazy"
        className={cn("object-cover", rounded, className)}
      />
    );
  }
  return (
    <div
      aria-hidden="true"
      className={cn(
        "grid place-items-center font-semibold text-white/90",
        rounded,
        className,
      )}
      style={{ background: coverGradient(seed) }}
    >
      <span className="opacity-80">{coverInitials(label)}</span>
    </div>
  );
}
