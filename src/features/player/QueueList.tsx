import { useMemo, useRef } from "react";
import { Play, X } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";
import { VirtualList } from "@/components/VirtualList";
import { Artwork } from "@/features/player/Artwork";
import type { ArtSource } from "@/lib/useTrackArtwork";
import { Switch } from "@/components/Switch";
import { cn } from "@/lib/cn";

const ROW_H = 68;

interface Row {
  item: QueueItem;
  /** Queue index (for removal). */
  qi: number;
  /** Position within the play order (for jump + highlight). */
  pos: number;
}

/** Where to resolve a queue item's cover art from, by source. */
function queueArt(item: QueueItem): ArtSource {
  const key = `${item.source}:${item.id}`;
  if (item.source === "phone" && item.device && item.phoneTrack) {
    return {
      key,
      source: "phone",
      deviceId: item.device.id,
      trackId: item.phoneTrack.id,
      hasArt: item.phoneTrack.hasArt,
    };
  }
  if (item.source === "cloud") {
    // The current track may carry a decoded cover (patched in from the engine's
    // now-playing event) — used directly. Every other row resolves its cover
    // lazily on demand (visible rows only, bounded cache + in-flight dedup in
    // useTrackArtwork), so a huge queue never holds thousands of covers.
    return {
      key,
      source: "cloud",
      cover: item.cover ?? null,
      cloudAccountId: item.cloud?.accountId,
      cloudFileId: item.cloud?.id,
      cloudName: item.cloud?.name,
    };
  }
  if (item.source === "ytmusic") {
    // The listing hands us the thumbnail URL up front, so it's already on the
    // item — nothing to resolve, unlike cloud. Without this the row fell to the
    // local case below, looked for a file path it has never had, and drew the
    // gradient while the same track showed its art everywhere else.
    return { key, source: "ytmusic", cover: item.cover ?? null };
  }
  // Local files read embedded art by path; radio has none (→ gradient).
  return { key, source: "local", path: item.track?.path ?? null };
}

/**
 * A queue row: album thumbnail + title/artist. The now-playing row is shown
 * statically at the top (green title, no controls); up-next rows reveal a play
 * overlay + remove button on hover and jump to that track when clicked.
 */
function QueueRow({
  row,
  nowPlaying = false,
  onJump,
  onRemove,
}: {
  row: Row;
  nowPlaying?: boolean;
  onJump: (pos: number) => void;
  onRemove: (qi: number) => void;
}) {
  const { item, qi, pos } = row;
  const seed = item.album?.trim() || item.title;
  return (
    <div
      onClick={() => onJump(pos)}
      className={cn(
        "group flex h-full cursor-pointer items-center gap-3 rounded-lg px-2 transition-colors",
        !nowPlaying && "hover:bg-surface-overlay",
      )}
    >
      <div className="relative size-12 shrink-0">
        <Artwork
          art={queueArt(item)}
          seed={seed}
          label={item.title}
          className="size-12 text-base"
          rounded="rounded-md"
        />
        {!nowPlaying && (
          <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
            <Play className="size-5 fill-white text-white" aria-hidden="true" />
          </span>
        )}
      </div>

      <div className="min-w-0 flex-1">
        <p
          className={cn(
            "truncate text-[15px] font-semibold leading-tight",
            nowPlaying ? "text-accent-strong" : "text-text",
          )}
        >
          {item.title}
        </p>
        <p className="mt-1 flex items-center gap-1.5 truncate text-[13px] text-text-muted">
          {item.autoAdded && (
            <span className="shrink-0 rounded-full border border-border-strong px-1.5 text-[10px] font-semibold uppercase tracking-wide text-text-faint">
              Radio
            </span>
          )}
          <span className="truncate">{item.artist ?? "Unknown artist"}</span>
        </p>
      </div>

      {!nowPlaying && (
        <button
          type="button"
          aria-label={`Remove ${item.title} from queue`}
          onClick={(e) => {
            e.stopPropagation();
            onRemove(qi);
          }}
          className="hidden size-7 shrink-0 place-items-center rounded-control text-text-faint hover:bg-surface hover:text-danger group-hover:grid"
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

/**
 * The play queue: a static "Now playing" card at the top and a scrollable
 * "Next up" list below. Reads the order straight off the engine store so it
 * always reflects what's queued (any mix of local / phone / cloud); the up-next
 * list is virtualized for very long queues.
 */
export function QueueList() {
  const queue = useEngineStore((s) => s.queue);
  const order = useEngineStore((s) => s.order);
  const orderPos = useEngineStore((s) => s.orderPos);
  const jumpTo = useEngineStore((s) => s.jumpTo);
  const removeFromQueue = useEngineStore((s) => s.removeFromQueue);
  const autoplay = useEngineStore((s) => s.state.playback.autoplay);
  const setAutoplay = useEngineStore((s) => s.setAutoplay);
  const scrollRef = useRef<HTMLDivElement>(null);

  const rows = useMemo<Row[]>(
    () =>
      order
        .map((qi, pos) => ({ item: queue[qi]!, qi, pos }))
        .filter((r): r is Row => Boolean(r.item)),
    [order, queue],
  );

  const current = useMemo(
    () => rows.find((r) => r.pos === orderPos) ?? null,
    [rows, orderPos],
  );
  const upcoming = useMemo(
    () => rows.filter((r) => r.pos > orderPos),
    [rows, orderPos],
  );

  if (rows.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-4">
        <p className="text-center text-sm text-text-muted">
          Nothing queued. Play something to fill the queue.
        </p>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Now playing — static. */}
      {current && (
        <div className="shrink-0 px-2 pt-3">
          <h3 className="mb-2 px-1 text-base font-bold tracking-tight">
            Now playing
          </h3>
          <div style={{ height: ROW_H }}>
            <QueueRow
              row={current}
              nowPlaying
              onJump={jumpTo}
              onRemove={removeFromQueue}
            />
          </div>
        </div>
      )}

      {/* Next up — header static, list scrolls. */}
      <div className="flex shrink-0 items-center justify-between px-3 pb-2 pt-4">
        <h3 className="text-base font-bold tracking-tight">Next up</h3>
        <label className="flex items-center gap-2 text-[13px] text-text-muted">
          Autoplay
          <Switch checked={autoplay} onChange={setAutoplay} label="Autoplay — keep similar tracks coming" />
        </label>
      </div>
      {upcoming.length === 0 ? (
        <p className="px-3 text-sm text-text-muted">Nothing up next.</p>
      ) : (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          <VirtualList
            items={upcoming}
            rowHeight={ROW_H}
            scrollRef={scrollRef}
            ariaLabel="Up next"
            getKey={(r) => `${r.qi}:${r.item.id}`}
            renderRow={(r) => (
              <QueueRow row={r} onJump={jumpTo} onRemove={removeFromQueue} />
            )}
          />
        </div>
      )}
    </div>
  );
}
