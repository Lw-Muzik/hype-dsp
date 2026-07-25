import { useState } from "react";
import { Clapperboard } from "lucide-react";
import { cn } from "@/lib/cn";

/**
 * A movie's poster (`archive.org/services/img/<id>`) with a graceful fallback
 * to a clapperboard glyph — mirrors `ChannelLogo` in `TvPanel.tsx`. Archive
 * doesn't guarantee every identifier has a poster image, so a 404 must read
 * as "no poster", never as a broken image icon.
 */
export function MoviePoster({
  src,
  title,
  className,
}: {
  src: string;
  title: string;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);
  if (failed) {
    return (
      <div className={cn("grid place-items-center bg-surface-overlay text-text-faint", className)}>
        <Clapperboard className="size-8" aria-hidden="true" />
      </div>
    );
  }
  return (
    <img
      src={src}
      alt={title}
      loading="lazy"
      onError={() => setFailed(true)}
      className={cn("bg-surface-overlay object-cover", className)}
    />
  );
}
