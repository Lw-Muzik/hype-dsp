import { useRef } from "react";
import { ChevronLeft, ChevronRight, Play } from "lucide-react";
import { Artwork } from "@/features/player/Artwork";

const reduceMotion =
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** One album card in the carousel. */
export interface CarouselAlbum {
  key: string;
  name: string;
  artist: string;
  path?: string | null;
  seed: string;
  /** Index into the full track list to start playback from. */
  index: number;
}

/**
 * A horizontal, scrollable strip of album cards (cover + name + artist). The
 * chevrons nudge the strip; cards play on click.
 */
export function AlbumCarousel({
  title,
  albums,
  onPlay,
}: {
  title: string;
  albums: CarouselAlbum[];
  onPlay: (index: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  if (albums.length === 0) return null;

  const nudge = (dir: 1 | -1) =>
    ref.current?.scrollBy({
      left: dir * 360,
      behavior: reduceMotion ? "auto" : "smooth",
    });

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">{title}</h3>
        {albums.length > 3 && (
          <div className="flex gap-1">
            <CarouselNav side="left" onClick={() => nudge(-1)} />
            <CarouselNav side="right" onClick={() => nudge(1)} />
          </div>
        )}
      </div>
      <div
        ref={ref}
        className="flex gap-4 overflow-x-auto pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {albums.map((a) => (
          <button
            key={a.key}
            type="button"
            onClick={() => onPlay(a.index)}
            title={`${a.name} — ${a.artist}`}
            className="group flex w-36 shrink-0 flex-col gap-2 text-left"
          >
            <div className="relative">
              <Artwork
                path={a.path}
                seed={a.seed}
                label={a.name}
                rounded="rounded-xl"
                className="aspect-square w-full shadow-md ring-1 ring-white/5 transition-transform group-hover:scale-[1.03]"
              />
              <span className="absolute inset-0 grid place-items-center rounded-xl bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
                <span className="grid size-10 place-items-center rounded-full bg-accent text-surface shadow-lg">
                  <Play className="size-5 fill-current" aria-hidden="true" />
                </span>
              </span>
            </div>
            <div className="min-w-0">
              <p className="truncate text-sm font-medium">{a.name}</p>
              <p className="truncate text-xs text-text-muted">{a.artist}</p>
            </div>
          </button>
        ))}
      </div>
    </section>
  );
}

function CarouselNav({
  side,
  onClick,
}: {
  side: "left" | "right";
  onClick: () => void;
}) {
  const Icon = side === "left" ? ChevronLeft : ChevronRight;
  return (
    <button
      type="button"
      aria-label={side === "left" ? "Scroll left" : "Scroll right"}
      onClick={onClick}
      className="grid size-7 place-items-center rounded-full border border-border text-text-muted transition-colors hover:border-border-strong hover:text-text"
    >
      <Icon className="size-4" aria-hidden="true" />
    </button>
  );
}
