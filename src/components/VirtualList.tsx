import { useLayoutEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { cn } from "@/lib/cn";

/**
 * A windowed (virtualized) list for very large collections: it reserves the
 * full scroll height but only mounts the rows currently in (or near) view, so a
 * 100k-item list renders ~30 DOM nodes and stays smooth. Rows are a fixed
 * height. It virtualizes against an ancestor scroll container (so the list can
 * live inside a page that scrolls as one), measuring its position with rAF-
 * throttled reads.
 */
export function VirtualList<T>({
  items,
  rowHeight,
  renderRow,
  getKey,
  scrollRef,
  overscan = 10,
  className,
  ariaLabel,
}: {
  items: T[];
  rowHeight: number;
  renderRow: (item: T, index: number) => ReactNode;
  getKey: (item: T, index: number) => string;
  /** The scrollable ancestor this list lives inside. */
  scrollRef: RefObject<HTMLElement | null>;
  overscan?: number;
  className?: string;
  ariaLabel?: string;
}) {
  const spacerRef = useRef<HTMLDivElement>(null);
  const [range, setRange] = useState({ start: 0, end: 0 });

  useLayoutEffect(() => {
    const scroller = scrollRef.current;
    const spacer = spacerRef.current;
    if (!scroller || !spacer) return;

    let raf = 0;
    const measure = () => {
      raf = 0;
      // How far the list's top has scrolled above the scroller's viewport top.
      const scrolledPast = Math.max(
        0,
        scroller.getBoundingClientRect().top - spacer.getBoundingClientRect().top,
      );
      const viewport = scroller.clientHeight || 0;
      const start = Math.max(0, Math.floor(scrolledPast / rowHeight) - overscan);
      const count = Math.ceil(viewport / rowHeight) + overscan * 2;
      const end = Math.min(items.length, start + count);
      setRange((prev) => (prev.start === start && prev.end === end ? prev : { start, end }));
    };
    const onScroll = () => {
      if (!raf) raf = requestAnimationFrame(measure);
    };

    measure();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(measure);
    ro.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
  }, [scrollRef, rowHeight, overscan, items.length]);

  const visible = items.slice(range.start, range.end);

  return (
    <div
      ref={spacerRef}
      role="list"
      aria-label={ariaLabel}
      aria-rowcount={items.length}
      className={cn("relative", className)}
      style={{ height: items.length * rowHeight }}
    >
      <div style={{ transform: `translateY(${range.start * rowHeight}px)` }}>
        {visible.map((item, i) => {
          const index = range.start + i;
          return (
            <div key={getKey(item, index)} role="listitem" style={{ height: rowHeight }}>
              {renderRow(item, index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
