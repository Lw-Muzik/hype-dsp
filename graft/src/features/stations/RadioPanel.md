# src/features/stations/RadioPanel.tsx

- Mode · type · L17-L17 — type Mode = "browse" | "africa" | "favorites";
- flag · function · L20-L24 — function flag(code: string): string
- RadioPanel · function · L49-L211 — function RadioPanel()
- openCountry · function · L88-L96 — openCountry = (c: RadioCountry)
- toggleFavorite · function · L98-L101 — toggleFavorite = (s: RadioStation)
- CountryGrid · function · L213-L245 — function CountryGrid({ countries, onPick, }: { countries: RadioCountry[]; onPick: (c: RadioCountry) => void; })
- StationProps · type · L247-L256 — type StationProps = { stations: RadioStation[]; loading: boolean; emptyLabel: string; layout: LayoutMode; nowPlaying: string | null; favIds: Set<string>; onPlay: (s: RadioStation) => void; onToggleFavorite: (s: RadioStation) => void; };
- stationSubtitle · function · L258-L260 — function stationSubtitle(s: RadioStation): string
- StationList · function · L262-L273 — function StationList(props: StationProps)
- StationRows · function · L276-L346 — function StationRows({ stations, nowPlaying, favIds, onPlay, onToggleFavorite, grid, }: StationProps & { grid?: boolean })
- FavStar · function · L348-L374 — function FavStar({ isFav, onClick, className, }: { isFav: boolean; onClick: () => void; className?: string; })
- StationLogo · function · L377-L400 — function StationLogo({ src, className }: { src: string | null; className?: string })
