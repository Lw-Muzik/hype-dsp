import { useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Play } from "lucide-react";
import { Artwork } from "@/features/player/Artwork";
import { cn } from "@/lib/cn";

/** One spotlighted album/track in the deck. */
export interface DeckItem {
  key: string;
  title: string;
  artist: string;
  /** Local path for embedded art (optional). */
  path?: string | null;
  /** Gradient seed for the art fallback. */
  seed: string;
  /** Index into the full track list to start playback from. */
  index: number;
}

const THROW_PX = 90; // drag distance that commits to the next/prev card
const reduceMotion =
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * The library's hero: a stack of album cards fanned like a hand, with the
 * front card swipeable (drag/throw, arrow keys, or the chevrons) to flip
 * through featured albums. Tapping the front card plays it. This deck is the
 * view's signature — everything around it stays quiet.
 */
export function AlbumDeck({
  items,
  onPlay,
}: {
  items: DeckItem[];
  onPlay: (index: number) => void;
}) {
  const [active, setActive] = useState(0);
  const [drag, setDrag] = useState(0);
  const [fly, setFly] = useState<-1 | 0 | 1>(0);
  const startX = useRef<number | null>(null);
  const dragging = useRef(false);

  const count = items.length;
  if (count === 0) return null;
  const at = (i: number) => ((i % count) + count) % count;

  const commit = (dir: 1 | -1) => {
    if (count <= 1) {
      setDrag(0);
      return;
    }
    setFly(dir === 1 ? -1 : 1); // fling toward the drag direction
    const after = () => {
      setActive((a) => at(a + dir));
      setFly(0);
      setDrag(0);
    };
    if (reduceMotion) after();
    else window.setTimeout(after, 220);
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (count <= 1) return;
    startX.current = e.clientX;
    dragging.current = false;
    (e.target as Element).setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (startX.current === null) return;
    const dx = e.clientX - startX.current;
    if (Math.abs(dx) > 4) dragging.current = true;
    setDrag(dx);
  };
  const onPointerUp = () => {
    const dx = drag;
    startX.current = null;
    if (Math.abs(dx) > THROW_PX) commit(dx < 0 ? 1 : -1);
    else setDrag(0);
  };

  // Front card transform: follow the finger while dragging, fling on release.
  const frontStyle: React.CSSProperties = fly
    ? {
        transform: `translateX(${fly * 480}px) rotate(${fly * 14}deg)`,
        opacity: 0,
        transition: "transform 220ms ease-in, opacity 220ms ease-in",
      }
    : {
        transform: `translateX(${drag}px) rotate(${drag / 22}deg)`,
        transition: startX.current === null ? "transform 260ms cubic-bezier(.22,1,.36,1)" : "none",
      };

  return (
    <section className="px-1" aria-roledescription="carousel" aria-label="Featured albums">
      <div className="group relative mx-auto h-52 w-full max-w-2xl">
        {/* Cards behind the front one, fanned out with depth. */}
        {Array.from({ length: Math.min(count - 1, 3) }, (_, k) => {
          const depth = k + 1; // 1 = nearest behind
          const item = items[at(active + depth)]!;
          const side = depth % 2 === 1 ? 1 : -1; // alternate the fan side
          const x = side * (26 + depth * 14);
          return (
            <DeckCard
              key={`${item.key}-bg`}
              item={item}
              rank={at(active + depth) + 1}
              style={{
                transform: `translateX(${x}px) scale(${1 - depth * 0.06}) rotate(${side * (depth * 2 + 2)}deg)`,
                zIndex: 10 - depth,
                opacity: 1 - depth * 0.18,
                filter: `brightness(${1 - depth * 0.18})`,
                transition: "transform 260ms cubic-bezier(.22,1,.36,1), opacity 260ms",
              }}
            />
          );
        })}

        {/* Front (interactive) card. */}
        <DeckCard
          item={items[active]!}
          rank={active + 1}
          interactive
          style={{ ...frontStyle, zIndex: 20 }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onActivate={() => {
            if (!dragging.current) onPlay(items[active]!.index);
          }}
        />

        {count > 1 && (
          <>
            <DeckNav side="left" onClick={() => commit(-1)} />
            <DeckNav side="right" onClick={() => commit(1)} />
          </>
        )}
      </div>

      {count > 1 && (
        <div className="mt-3 flex justify-center gap-1.5">
          {items.map((it, i) => (
            <button
              key={it.key}
              type="button"
              aria-label={`Show ${it.title}`}
              aria-current={i === active}
              onClick={() => setActive(i)}
              className={cn(
                "h-1.5 rounded-full transition-all",
                i === active ? "w-5 bg-accent" : "w-1.5 bg-border-strong hover:bg-text-faint",
              )}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function DeckCard({
  item,
  rank,
  interactive = false,
  style,
  onActivate,
  ...pointer
}: {
  item: DeckItem;
  rank: number;
  interactive?: boolean;
  style?: React.CSSProperties;
  onActivate?: () => void;
} & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      {...pointer}
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      aria-label={interactive ? `Play ${item.title} by ${item.artist}` : undefined}
      onClick={interactive ? onActivate : undefined}
      onKeyDown={
        interactive
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onActivate?.();
              }
            }
          : undefined
      }
      style={style}
      className={cn(
        "group absolute inset-0 overflow-hidden rounded-2xl shadow-xl ring-1 ring-white/10 select-none",
        interactive ? "cursor-grab active:cursor-grabbing" : "pointer-events-none",
      )}
    >
      <Artwork
        path={item.path}
        seed={item.seed}
        label={item.title}
        rounded="rounded-2xl"
        className="absolute inset-0 size-full"
      />
      {/* Legibility scrim + content. */}
      <div className="absolute inset-0 bg-gradient-to-t from-black/80 via-black/25 to-black/40" aria-hidden="true" />
      <span className="absolute left-4 top-3 text-3xl font-black leading-none text-white/90 drop-shadow">
        #{rank}
      </span>
      {interactive && (
        <span className="absolute right-4 top-3 grid size-10 place-items-center rounded-full bg-accent text-surface shadow-lg transition-transform group-hover:scale-105">
          <Play className="size-5 fill-current" aria-hidden="true" />
        </span>
      )}
      <div className="absolute inset-x-4 bottom-3 flex items-end justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[10px] font-semibold uppercase tracking-widest text-white/60">Album</p>
          <p className="truncate text-lg font-bold text-white">{item.title}</p>
        </div>
        <div className="min-w-0 max-w-[45%] text-right">
          <p className="text-[10px] font-semibold uppercase tracking-widest text-white/60">Artist</p>
          <p className="truncate text-sm font-semibold text-white/90">{item.artist}</p>
        </div>
      </div>
    </div>
  );
}

function DeckNav({ side, onClick }: { side: "left" | "right"; onClick: () => void }) {
  const Icon = side === "left" ? ChevronLeft : ChevronRight;
  return (
    <button
      type="button"
      aria-label={side === "left" ? "Previous album" : "Next album"}
      onClick={onClick}
      className={cn(
        "absolute top-1/2 z-30 grid size-9 -translate-y-1/2 place-items-center rounded-full",
        "bg-black/45 text-white opacity-0 transition-opacity hover:bg-black/65",
        "focus-visible:opacity-100 group-hover:opacity-100",
        side === "left" ? "left-1" : "right-1",
      )}
    >
      <Icon className="size-5" aria-hidden="true" />
    </button>
  );
}
