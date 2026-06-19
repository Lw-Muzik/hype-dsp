import { useTrackArtwork } from "@/lib/useTrackArtwork";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";

/**
 * A track/album cover: the file's real embedded art when present (lazily
 * fetched by path and cached), otherwise a deterministic gradient with
 * initials. Decorative — the surrounding control carries the accessible name.
 */
export function Artwork({
  path,
  seed,
  label,
  className,
  rounded = "rounded-lg",
}: {
  /** Local file path to lazy-load embedded art from (omit for non-local). */
  path?: string | null;
  /** Stable seed for the gradient fallback (album or title). */
  seed: string;
  /** Source for the fallback initials (usually the title). */
  label: string;
  className?: string;
  /** Tailwind rounding utility (so callers can match the surrounding shape). */
  rounded?: string;
}) {
  const cover = useTrackArtwork(path ?? null);
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
