# src/features/movies/MoviesView.tsx

- movieKey · function · L23-L25 — function movieKey(m: Movie): string
- MoviesView · function · L40-L83 — function MoviesView()
- SearchBar · function · L88-L137 — function SearchBar()
- onType · function · L103-L110 — onType = (v: string)
- SearchResults · function · L139-L191 — function SearchResults({ scrollRef }: { scrollRef: RefObject<HTMLDivElement | null> })
- Shelf · function · L197-L247 — function Shelf({ collection }: { collection: MovieCollection })
- YtShelf · function · L252-L302 — function YtShelf({ channel }: { channel: YtMovieChannel })
- MovieCard · function · L310-L351 — function MovieCard({ movie, onClick, fixedWidth, }: { movie: Movie; onClick: () => void; fixedWidth?: boolean; })
- Centered · function · L353-L383 — function Centered({ icon: Icon, title, body, action, danger = false, }: { icon: typeof CircleAlert; title: string; body: string; action?: ReactNode; danger?: boolean; })
