import { useEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { CircleAlert, Loader2, RotateCw, Search, X } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { VirtualGrid } from "@/components/VirtualGrid";
import { MOVIE_COLLECTIONS, YT_MOVIE_CHANNELS, useMoviesStore } from "@/stores/movies";
import type { MovieCollection, YtMovieChannel } from "@/stores/movies";
import type { Movie } from "@/lib/types";
import { cn } from "@/lib/cn";
import { MoviePoster } from "./MoviePoster";
import { MovieDetailView } from "./MovieDetailView";
import { formatDurationLabel } from "./duration";

const GRID_MIN_COL = 150;
const GRID_TEXT_H = 54;
/** How many placeholder cards a loading shelf shows — enough to fill a
 *  typical shelf width without being wasteful. */
const SHELF_SKELETON_COUNT = 6;

/** A movie card's stable identity, whichever backend it came from. */
function movieKey(m: Movie): string {
  return m.source === "youtube" ? m.videoId : m.identifier;
}

/**
 * Movies — modern films from licensed distributor YouTube channels, played
 * through the official iframe embed, alongside public-domain and
 * Creative-Commons "Classics" from the Internet Archive. No pirate source
 * either way.
 *
 * Three screens, exactly like Explore: shelves (one per YouTube channel and
 * per confirmed Archive collection, each fetched lazily), a search grid, and
 * a movie's detail/player. Search stays Archive-only — a YouTube channel's
 * uploads have no keyless search, only a listing (that needs the YouTube
 * Data API) — so nothing here is cached beyond the session either way; each
 * shelf/search is a live read.
 */
export function MoviesView() {
  const route = routeById("movies");
  const opened = useMoviesStore((s) => s.opened);
  const query = useMoviesStore((s) => s.query);

  // The one scroll container every screen lives in — the search grid
  // virtualizes against it, same as Explore's opened playlist.
  const scrollRef = useRef<HTMLDivElement>(null);
  const searching = query.length > 0;

  return (
    <div className="mx-auto flex h-full w-full max-w-5xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      {!opened && <SearchBar />}

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
        {opened ? (
          // Keyed by source-aware identity so switching movies always starts
          // from a clean "not playing" state.
          <MovieDetailView key={movieKey(opened)} />
        ) : searching ? (
          <SearchResults scrollRef={scrollRef} />
        ) : (
          <div className="flex flex-col gap-8 pb-4">
            {YT_MOVIE_CHANNELS.map((c) => (
              <YtShelf key={c.id} channel={c} />
            ))}

            <div>
              <h2 className="text-base font-semibold">Classics</h2>
              <p className="text-xs text-text-faint">
                Public-domain and Creative-Commons films from the Internet Archive.
              </p>
            </div>
            {MOVIE_COLLECTIONS.map((c) => (
              <Shelf key={c.id} collection={c} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** The debounced search box. Typing (not Enter) drives the search — Archive's
 *  search is one HTTP GET, not a per-keystroke-expensive YouTube call, so a
 *  short debounce is enough to avoid firing on every character. */
function SearchBar() {
  const query = useMoviesStore((s) => s.query);
  const search = useMoviesStore((s) => s.search);
  const clearSearch = useMoviesStore((s) => s.clearSearch);

  // The box holds what's typed; the store holds what was *asked* — the same
  // split Explore's search box makes, so the field stays editable mid-debounce.
  const [text, setText] = useState(query);
  const debounce = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!query) setText("");
  }, [query]);
  useEffect(() => () => window.clearTimeout(debounce.current), []);

  const onType = (v: string) => {
    setText(v);
    window.clearTimeout(debounce.current);
    debounce.current = window.setTimeout(() => {
      if (v.trim()) search(v);
      else clearSearch();
    }, 350);
  };

  return (
    <div className="flex items-center gap-2 rounded-control border border-border bg-surface-raised px-3 focus-within:border-accent">
      <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
      <input
        value={text}
        onChange={(e) => onType(e.target.value)}
        placeholder="Search public-domain and Creative-Commons films…"
        aria-label="Search movies"
        className="min-w-0 flex-1 bg-transparent py-2 text-sm outline-none placeholder:text-text-faint"
      />
      {text && (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => {
            setText("");
            clearSearch();
          }}
          className="grid size-5 shrink-0 place-items-center rounded-full text-text-faint transition-colors hover:bg-surface-overlay hover:text-text"
        >
          <X className="size-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

function SearchResults({ scrollRef }: { scrollRef: RefObject<HTMLDivElement | null> }) {
  const query = useMoviesStore((s) => s.query);
  const results = useMoviesStore((s) => s.results);
  const searchLoad = useMoviesStore((s) => s.searchLoad);
  const searchError = useMoviesStore((s) => s.searchError);
  const search = useMoviesStore((s) => s.search);
  const open = useMoviesStore((s) => s.open);

  if (searchLoad === "loading") {
    return (
      <div className="flex items-center justify-center gap-2 py-16 text-sm text-text-muted">
        <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        Searching for “{query}”…
      </div>
    );
  }
  if (searchLoad === "error") {
    return (
      <Centered
        icon={CircleAlert}
        danger
        title="Couldn't search the Internet Archive"
        body={searchError ?? "Something went wrong."}
        action={
          <Button variant="primary" onClick={() => search(query)}>
            <RotateCw className="size-4" aria-hidden="true" />
            Retry
          </Button>
        }
      />
    );
  }
  if (results.length === 0) {
    return (
      <Centered
        icon={Search}
        title="No results"
        body={`Nothing in the Internet Archive's film collections matches "${query}".`}
      />
    );
  }
  return (
    <VirtualGrid
      items={results}
      minColWidth={GRID_MIN_COL}
      textHeight={GRID_TEXT_H}
      scrollRef={scrollRef}
      ariaLabel="Search results"
      getKey={movieKey}
      renderCell={(m) => <MovieCard movie={m} onClick={() => open(m)} />}
    />
  );
}

/** One collection's popular films, a lazily-loaded horizontal shelf — same
 *  fetch-on-mount-and-keep pattern as Explore's category shelves. A shelf
 *  that fails to load reports its own error so one dead collection never
 *  blanks the rest of the page. */
function Shelf({ collection }: { collection: MovieCollection }) {
  const shelf = useMoviesStore((s) => s.shelves[collection.id]);
  const ensureShelf = useMoviesStore((s) => s.ensureShelf);
  const retryShelf = useMoviesStore((s) => s.retryShelf);
  const open = useMoviesStore((s) => s.open);

  useEffect(() => {
    ensureShelf(collection.id);
  }, [ensureShelf, collection.id]);

  const load = shelf?.load ?? "idle";
  const movies = shelf?.movies ?? [];

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-medium text-text-muted">{collection.label}</h2>
      {load === "error" ? (
        <div className="flex items-center gap-3 rounded-control border border-border bg-surface-raised px-4 py-3 text-sm text-text-muted">
          <CircleAlert className="size-4 shrink-0 text-danger" aria-hidden="true" />
          <span className="flex-1">{shelf?.error ?? "Couldn't load this shelf."}</span>
          <button
            type="button"
            onClick={() => retryShelf(collection.id)}
            className="flex shrink-0 items-center gap-1.5 transition-colors hover:text-text"
          >
            <RotateCw className="size-3.5" aria-hidden="true" />
            Retry
          </button>
        </div>
      ) : load === "loading" || load === "idle" ? (
        <div className="flex gap-3 overflow-x-auto pb-2">
          {Array.from({ length: SHELF_SKELETON_COUNT }).map((_, i) => (
            <div key={i} className="flex w-36 shrink-0 flex-col gap-2">
              <div className="aspect-[2/3] w-full animate-pulse rounded-card bg-surface-overlay" />
              <div className="h-3 w-full animate-pulse rounded bg-surface-overlay" />
              <div className="h-3 w-1/2 animate-pulse rounded bg-surface-overlay" />
            </div>
          ))}
        </div>
      ) : movies.length === 0 ? (
        <p className="text-sm text-text-faint">Nothing here right now.</p>
      ) : (
        <div className="flex gap-3 overflow-x-auto pb-2">
          {movies.map((m) => (
            <MovieCard key={movieKey(m)} movie={m} onClick={() => open(m)} fixedWidth />
          ))}
        </div>
      )}
    </section>
  );
}

/** One YouTube distributor channel's full-length uploads — the same lazy
 *  shelf pattern as `Shelf`, keyed by channel id and backed by
 *  `ensureYtShelf`/`retryYtShelf` instead of the Archive collection actions. */
function YtShelf({ channel }: { channel: YtMovieChannel }) {
  const shelf = useMoviesStore((s) => s.ytShelves[channel.id]);
  const ensureYtShelf = useMoviesStore((s) => s.ensureYtShelf);
  const retryYtShelf = useMoviesStore((s) => s.retryYtShelf);
  const open = useMoviesStore((s) => s.open);

  useEffect(() => {
    ensureYtShelf(channel.id);
  }, [ensureYtShelf, channel.id]);

  const load = shelf?.load ?? "idle";
  const movies = shelf?.movies ?? [];

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-medium text-text-muted">{channel.label}</h2>
      {load === "error" ? (
        <div className="flex items-center gap-3 rounded-control border border-border bg-surface-raised px-4 py-3 text-sm text-text-muted">
          <CircleAlert className="size-4 shrink-0 text-danger" aria-hidden="true" />
          <span className="flex-1">{shelf?.error ?? "Couldn't load this channel."}</span>
          <button
            type="button"
            onClick={() => retryYtShelf(channel.id)}
            className="flex shrink-0 items-center gap-1.5 transition-colors hover:text-text"
          >
            <RotateCw className="size-3.5" aria-hidden="true" />
            Retry
          </button>
        </div>
      ) : load === "loading" || load === "idle" ? (
        <div className="flex gap-3 overflow-x-auto pb-2">
          {Array.from({ length: SHELF_SKELETON_COUNT }).map((_, i) => (
            <div key={i} className="flex w-52 shrink-0 flex-col gap-2">
              <div className="aspect-video w-full animate-pulse rounded-card bg-surface-overlay" />
              <div className="h-3 w-full animate-pulse rounded bg-surface-overlay" />
              <div className="h-3 w-1/2 animate-pulse rounded bg-surface-overlay" />
            </div>
          ))}
        </div>
      ) : movies.length === 0 ? (
        <p className="text-sm text-text-faint">Nothing here right now.</p>
      ) : (
        <div className="flex gap-3 overflow-x-auto pb-2">
          {movies.map((m) => (
            <MovieCard key={movieKey(m)} movie={m} onClick={() => open(m)} fixedWidth />
          ))}
        </div>
      )}
    </section>
  );
}

/** One movie card, either backend. Fixed-width in a shelf's horizontal row
 *  (portrait 2:3 for an Archive poster, 16:9 for a YouTube thumbnail); fills
 *  its cell as a square in the virtualized search grid (Archive-only, so
 *  always the 2:3 case in practice) — `VirtualGrid` windows against a fixed
 *  `cellHeight = cellWidth + textHeight`, the same constraint that makes the
 *  TV channel grid's tiles square rather than poster-shaped. */
function MovieCard({
  movie,
  onClick,
  fixedWidth,
}: {
  movie: Movie;
  onClick: () => void;
  fixedWidth?: boolean;
}) {
  const isYoutube = movie.source === "youtube";
  const subtitle =
    movie.source === "youtube"
      ? [movie.channelTitle ?? "YouTube", formatDurationLabel(movie.durationSec)].filter(Boolean).join(" · ")
      : (movie.year ?? "Year unknown");
  const titleAttr =
    movie.source === "archive" && movie.year ? `${movie.title} (${movie.year})` : movie.title;

  return (
    <button
      type="button"
      onClick={onClick}
      title={titleAttr}
      className={cn(
        "group flex h-full flex-col gap-2 rounded-lg p-1.5 text-left transition-colors hover:bg-surface-raised",
        fixedWidth && (isYoutube ? "w-52 shrink-0" : "w-36 shrink-0"),
      )}
    >
      <MoviePoster
        src={movie.posterUrl}
        title={movie.title}
        className={cn(
          "w-full overflow-hidden rounded-control",
          fixedWidth ? (isYoutube ? "aspect-video" : "aspect-[2/3]") : "aspect-square",
        )}
      />
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{movie.title}</p>
        <p className="truncate text-xs text-text-muted">{subtitle}</p>
      </div>
    </button>
  );
}

function Centered({
  icon: Icon,
  title,
  body,
  action,
  danger = false,
}: {
  icon: typeof CircleAlert;
  title: string;
  body: string;
  action?: ReactNode;
  danger?: boolean;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div
        className={cn(
          "grid size-14 place-items-center rounded-2xl ring-1",
          danger ? "bg-danger/10 ring-danger/30" : "bg-surface-raised ring-border",
        )}
      >
        <Icon className={cn("size-7", danger ? "text-danger" : "text-text-faint")} aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">{title}</p>
        <p className="mt-1 max-w-sm text-sm text-text-muted">{body}</p>
      </div>
      {action}
    </div>
  );
}
