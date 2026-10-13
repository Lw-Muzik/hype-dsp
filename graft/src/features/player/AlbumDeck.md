# src/features/player/AlbumDeck.tsx

- DeckItem · interface · L9-L19 — interface DeckItem
- AlbumDeck · function · L29-L165 — function AlbumDeck({ items, onPlay, }: { items: DeckItem[]; onPlay: (index: number) => void; })
- at · function · L44-L44 — at = (i: number)
- commit · function · L46-L59 — commit = (dir: 1 | -1)
- after · function · L52-L56 — after = ()
- onPointerDown · function · L61-L66 — onPointerDown = (e: React.PointerEvent)
- onPointerMove · function · L67-L72 — onPointerMove = (e: React.PointerEvent)
- onPointerUp · function · L73-L78 — onPointerUp = ()
- DeckCard · function · L167-L233 — function DeckCard({ item, rank, interactive = false, style, onActivate, ...pointer }: { item: DeckItem; rank: number; interactive?: boolean; style?: React.CSSProperties; onActivate?: () => void; } & React.HTMLAttributes<HTMLDivElement>)
- DeckNav · function · L235-L252 — function DeckNav({ side, onClick }: { side: "left" | "right"; onClick: () => void })
