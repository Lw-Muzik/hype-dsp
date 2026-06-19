import { useEffect, useMemo, useRef } from "react";
import { ListMusic, Play, X } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import { VirtualList } from "@/components/VirtualList";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

const ROW_H = 52;

/**
 * The play queue: a right-side drawer listing the current play order with the
 * now-playing track highlighted. Click a track to jump to it, or remove one.
 * Reads the order straight off the engine store so it always reflects what's
 * actually queued (any mix of local / phone / cloud).
 */
export function QueueDrawer() {
  const open = useUiStore((s) => s.queueOpen);
  const close = useUiStore((s) => s.closeQueue);
  const queue = useEngineStore((s) => s.queue);
  const order = useEngineStore((s) => s.order);
  const orderPos = useEngineStore((s) => s.orderPos);
  const jumpTo = useEngineStore((s) => s.jumpTo);
  const removeFromQueue = useEngineStore((s) => s.removeFromQueue);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, close]);

  // The play order, resolved to items + their queue index + order position.
  const rows = useMemo(
    () =>
      order
        .map((qi, pos) => ({ item: queue[qi], qi, pos }))
        .filter((r): r is { item: NonNullable<(typeof queue)[number]>; qi: number; pos: number } =>
          Boolean(r.item),
        ),
    [order, queue],
  );

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={close}
        aria-hidden="true"
        className={cn(
          "fixed inset-0 z-40 bg-black/40 transition-opacity duration-200",
          open ? "opacity-100" : "pointer-events-none opacity-0",
        )}
      />
      {/* Panel */}
      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Play queue"
        className={cn(
          "fixed right-0 top-0 z-50 flex h-full w-80 flex-col border-l border-border bg-surface-raised shadow-2xl transition-transform duration-200",
          open ? "translate-x-0" : "translate-x-full",
        )}
      >
        <div className="flex h-14 shrink-0 items-center justify-between border-b border-border px-4">
          <div className="flex items-center gap-2">
            <ListMusic className="size-4 text-text-muted" aria-hidden="true" />
            <h2 className="text-sm font-semibold">Queue</h2>
            <span className="text-xs text-text-faint">{rows.length}</span>
          </div>
          <button
            type="button"
            aria-label="Close queue"
            onClick={close}
            className="grid size-7 place-items-center rounded-control text-text-faint transition-colors hover:bg-surface-overlay hover:text-text"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        </div>

        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto p-2">
          {rows.length === 0 ? (
            <p className="px-2 py-10 text-center text-sm text-text-muted">
              Nothing queued. Play something to fill the queue.
            </p>
          ) : (
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
          )}
        </div>
      </aside>
    </>
  );
}
