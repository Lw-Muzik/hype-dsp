import { useMemo, useRef } from "react";
import { Play, X } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { VirtualList } from "@/components/VirtualList";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

const ROW_H = 52;

/**
 * The current play order with the now-playing track highlighted. Click a track
 * to jump to it, or remove one. Reads the order straight off the engine store
 * so it always reflects what's queued (any mix of local / phone / cloud), and
 * is virtualized for very long queues.
 */
export function QueueList() {
  const queue = useEngineStore((s) => s.queue);
  const order = useEngineStore((s) => s.order);
  const orderPos = useEngineStore((s) => s.orderPos);
  const jumpTo = useEngineStore((s) => s.jumpTo);
  const removeFromQueue = useEngineStore((s) => s.removeFromQueue);
  const scrollRef = useRef<HTMLDivElement>(null);

  const rows = useMemo(
    () =>
      order
        .map((qi, pos) => ({ item: queue[qi], qi, pos }))
        .filter(
          (r): r is { item: NonNullable<(typeof queue)[number]>; qi: number; pos: number } =>
            Boolean(r.item),
        ),
    [order, queue],
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
    <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto p-2">
      <VirtualList
        items={rows}
        rowHeight={ROW_H}
        scrollRef={scrollRef}
        ariaLabel="Queued tracks"
        getKey={(r) => `${r.qi}:${r.item.id}`}
        renderRow={(r) => {
          const playing = r.pos === orderPos;
          return (
            <div
              onClick={() => jumpTo(r.pos)}
              className={cn(
                "group flex h-full cursor-pointer items-center gap-2.5 rounded-control px-2 transition-colors hover:bg-surface-overlay",
                playing && "bg-accent-muted/40",
              )}
            >
              <span className="grid w-5 shrink-0 place-items-center text-xs tabular-nums text-text-faint">
                {playing ? (
                  <Play className="size-3.5 fill-current text-accent-strong" aria-hidden="true" />
                ) : (
                  r.pos + 1
                )}
              </span>
              <div className="min-w-0 flex-1">
                <p
                  className={cn(
                    "truncate text-sm font-medium",
                    playing && "text-accent-strong",
                  )}
                >
                  {r.item.title}
                </p>
                <p className="truncate text-xs text-text-muted">
                  {r.item.artist ?? "Unknown artist"}
                </p>
              </div>
              {r.item.durationSecs != null && (
                <span className="shrink-0 text-xs tabular-nums text-text-faint group-hover:hidden">
                  {formatTime(r.item.durationSecs)}
                </span>
              )}
              <button
                type="button"
                aria-label={`Remove ${r.item.title} from queue`}
                onClick={(e) => {
                  e.stopPropagation();
                  removeFromQueue(r.qi);
                }}
                className="hidden size-7 shrink-0 place-items-center rounded-control text-text-faint hover:bg-surface hover:text-danger group-hover:grid"
              >
                <X className="size-4" aria-hidden="true" />
              </button>
            </div>
          );
        }}
      />
    </div>
  );
}
