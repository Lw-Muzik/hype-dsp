# src/components/VirtualList.tsx

- VirtualList · function · L13-L110 — function VirtualList<T>({ items, rowHeight, renderRow, getKey, scrollRef, overscan = 10, className, ariaLabel, }: { items: T[]; rowHeight: number; renderRow: (item: T, index: number) => ReactNode; getKey: (item: T, index: number) => string; /** The scrollable ancestor this list lives inside. */ scrollRef: RefObject<HTMLElement | null>; overscan?: number; className?: string; ariaLabel?: string; })
- measure · function · L44-L56 — measure = ()
- onScroll · function · L62-L68 — onScroll = ()
