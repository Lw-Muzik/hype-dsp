import { useEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { cn } from "@/lib/cn";

/**
 * A windowed (virtualized) responsive grid for very large collections. It
 * measures the container width to choose a column count and a *square* cell
 * size, then mounts only the rows currently in view — so a grid of 100k cards
 * stays as light as the list. Virtualizes against an ancestor scroll container
 * (timer-throttled, passive effect) exactly like {@link VirtualList}.
 */
export function VirtualGrid<T>({
  items,
  minColWidth,
  textHeight,
  gap = 16,
  renderCell,
  getKey,
  scrollRef,
  overscanRows = 3,
  className,
  ariaLabel,
}: {
  items: T[];
  /** Smallest acceptable card width (drives the column count). */
  minColWidth: number;
  /** Fixed height reserved below the square cover (title + subtitle). */
  textHeight: number;
  gap?: number;
  renderCell: (item: T, index: number) => ReactNode;
  getKey: (item: T, index: number) => string;
  scrollRef: RefObject<HTMLElement | null>;
  overscanRows?: number;
  className?: string;
  ariaLabel?: string;
}) {
  const spacerRef = useRef<HTMLDivElement>(null);
  const [cols, setCols] = useState(1);
  const [cellHeight, setCellHeight] = useState(minColWidth + textHeight);
  const [range, setRange] = useState({ startRow: 0, endRow: 0 });

  useEffect(() => {
    const scroller = scrollRef.current;
    const spacer = spacerRef.current;
    if (!scroller || !spacer) return;

    let cancelled = false;
    const measure = () => {
      if (cancelled) return;
      const width = spacer.clientWidth || 0;
      const c = Math.max(1, Math.floor((width + gap) / (minColWidth + gap)));
      const cellW = (width - (c - 1) * gap) / c;
      const cellH = Math.max(1, Math.round(cellW + textHeight));
      const stride = cellH + gap;
      const scrolledPast = Math.max(
        0,
        scroller.getBoundingClientRect().top - spacer.getBoundingClientRect().top,
      );
      const viewport = scroller.clientHeight || 0;
      const startRow = Math.max(0, Math.floor(scrolledPast / stride) - overscanRows);
      const visRows = Math.ceil(viewport / stride) + overscanRows * 2;
      const totalRows = Math.ceil(items.length / c);
      const endRow = Math.min(totalRows, startRow + visRows);
      setCols((p) => (p === c ? p : c));
      setCellHeight((p) => (p === cellH ? p : cellH));
      setRange((p) => (p.startRow === startRow && p.endRow === endRow ? p : { startRow, endRow }));
    };

    let timer = 0;
    const onScroll = () => {
      if (timer) return;
      timer = window.setTimeout(() => {
        timer = 0;
        measure();
      }, 16);
    };

    measure();
    const t0 = window.setTimeout(measure, 0);
    const t1 = window.setTimeout(measure, 250);
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(measure);
    ro.observe(scroller);
    ro.observe(spacer);
    return () => {
      cancelled = true;
      scroller.removeEventListener("scroll", onScroll);
      ro.disconnect();
      window.clearTimeout(t0);
      window.clearTimeout(t1);
      if (timer) window.clearTimeout(timer);
    };
  }, [scrollRef, minColWidth, textHeight, gap, items.length, overscanRows]);

  const stride = cellHeight + gap;
  const totalRows = Math.ceil(items.length / cols);
  const rows: number[] = [];
  for (let r = range.startRow; r < range.endRow; r++) rows.push(r);

  return (
    <div
      ref={spacerRef}
      role="list"
      aria-label={ariaLabel}
      aria-rowcount={items.length}
      className={cn("relative", className)}
      style={{ height: Math.max(0, totalRows * stride - gap) }}
    >
      <div
        style={{
          transform: `translateY(${range.startRow * stride}px)`,
          display: "grid",
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gap,
        }}
      >
        {rows.flatMap((r) => {
          const cells: ReactNode[] = [];
          for (let c = 0; c < cols; c++) {
            const idx = r * cols + c;
            if (idx >= items.length) break;
            const item = items[idx]!;
            cells.push(
              <div key={getKey(item, idx)} role="listitem" style={{ height: cellHeight }}>
                {renderCell(item, idx)}
              </div>,
            );
          }
          return cells;
        })}
      </div>
    </div>
  );
}
