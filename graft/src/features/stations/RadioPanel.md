# src/features/stations/RadioPanel.tsx

- Mode · type · L18-L18 — type Mode = "browse" | "africa" | "favorites";
- flag · function · L21-L25 — function flag(code: string): string
- RadioPanel · function · L50-L220 — function RadioPanel()
- playRadio · function · L54-L59 — playRadio = (station: RadioStation)
- openCountry · function · L97-L105 — openCountry = (c: RadioCountry)
- toggleFavorite · function · L107-L110 — toggleFavorite = (s: RadioStation)
- CountryGrid · function · L222-L254 — function CountryGrid({ countries, onPick, }: { countries: RadioCountry[]; onPick: (c: RadioCountry) => void; })
- StationProps · type · L256-L265 — type StationProps = { stations: RadioStation[]; loading: boolean; emptyLabel: string; layout: LayoutMode; nowPlaying: string | null; favIds: Set<string>; onPlay: (s: RadioStation) => void; onToggleFavorite: (s: RadioStation) => void; };
- stationSubtitle · function · L267-L269 — function stationSubtitle(s: RadioStation): string
- StationList · function · L271-L282 — function StationList(props: StationProps)
- StationRows · function · L285-L355 — function StationRows({ stations, nowPlaying, favIds, onPlay, onToggleFavorite, grid, }: StationProps & { grid?: boolean })
- FavStar · function · L357-L383 — function FavStar({ isFav, onClick, className, }: { isFav: boolean; onClick: () => void; className?: string; })
- StationLogo · function · L386-L409 — function StationLogo({ src, className }: { src: string | null; className?: string })
