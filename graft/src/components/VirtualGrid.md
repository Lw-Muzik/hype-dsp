# src/components/VirtualGrid.tsx

- VirtualGrid · function · L12-L134 — function VirtualGrid<T>({ items, minColWidth, textHeight, gap = 16, renderCell, getKey, scrollRef, overscanRows = 3, className, ariaLabel, }: { items: T[]; /** Smallest acceptable card width (drives the column count). */ minColWidth: number; /** Fixed height reserved below the square cover (title + subtitle). */ textHeight: number; gap?: number; renderCell: (item: T, index: number) => ReactNode; getKey: (item: T, index: number) => string; scrollRef: RefObject<HTMLElement | null>; overscanRows?: number; className?: string; ariaLabel?: string; })
- measure · function · L48-L67 — measure = ()
- onScroll · function · L70-L76 — onScroll = ()
