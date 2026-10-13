# src/features/stations/TvPanel.tsx

- Mode · type · L24-L24 — type Mode = "browse" | "country" | "category" | "favorites";
- flag · function · L33-L37 — function flag(code: string): string
- TvPanel · function · L44-L318 — function TvPanel({ active }: { active: boolean })
- openCountry · function · L124-L136 — openCountry = (c: TvCountry)
- openCategory · function · L138-L150 — openCategory = (c: TvCategory)
- toggleFavorite · function · L152-L155 — toggleFavorite = (c: TvChannel)
- BackButton · function · L320-L331 — function BackButton({ onClick, children }: { onClick: () => void; children: ReactNode })
- PickGrid · function · L334-L368 — function PickGrid({ items, onPick, empty, }: { items: { key: string; label: string; badge?: string }[]; onPick: (key: string) => void; empty: string; })
- ChannelProps · type · L370-L383 — type ChannelProps = { channels: TvChannel[]; loading: boolean; emptyLabel: string; layout: LayoutMode; watchingId: string | null; favIds: Set<string>; onPlay: (c: TvChannel) => void; onToggleFavorite: (c: TvChannel) => void; /** Probe verdict per channel, for dimming the dead ones. */ healthOf: (c: TvChannel) => Health; /** The scroll container the list/grid virtualizes against. */ scrollRef: RefObject<HTMLDivElement | null>; };
- ChannelCollection · function · L392-L403 — function ChannelCollection(props: ChannelProps)
- subtitleOf · function · L405-L407 — function subtitleOf(c: TvChannel): string
- ChannelList · function · L409-L442 — function ChannelList({ channels, watchingId, favIds, onPlay, onToggleFavorite, healthOf, scrollRef }: ChannelProps)
- ChannelGrid · function · L444-L486 — function ChannelGrid({ channels, watchingId, favIds, onPlay, onToggleFavorite, healthOf, scrollRef }: ChannelProps)
- FavButton · function · L488-L514 — function FavButton({ isFav, onClick, className, }: { isFav: boolean; onClick: () => void; className?: string; })
- ChannelLogo · function · L517-L543 — function ChannelLogo({ src, className, contain, }: { src: string | null; className?: string; contain?: boolean; })
