import { useEffect, useRef, useState } from "react";
import {
  ChevronLeft,
  CircleAlert,
  Disc3,
  ListMusic,
  Loader2,
  Mic2,
  Play,
  RotateCw,
  Search,
  SquarePlay,
  Video as VideoIcon,
  X,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { Artwork } from "@/features/player/Artwork";
import { TrackRow, TRACK_ROW_H } from "@/features/player/TrackRow";
import { SEARCH_FILTERS, useExploreStore } from "@/stores/explore";
import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import type { ExploreItem, ExploreShelf } from "@/lib/types";
import { cn } from "@/lib/cn";

/**
 * Explore — YouTube Music's own catalog.
 *
 * Three screens: the mood/genre categories, one category's shelves, and one
 * opened playlist/album's tracks. Everything is fetched on click and nothing is
 * kept: this is the live catalog, and a cached copy of it would just be a worse
 * Library.
 *
 * Opening a tile *lists* its tracks rather than playing them. Browsing is how
 * you find out what something is — a hundred tracks starting unannounced on a
 * single click answers a question you hadn't asked yet.
 */
export function ExploreView() {
  const route = routeById("explore");
  const setRoute = useUiStore((s) => s.setRoute);

  const signedIn = useExploreStore((s) => s.signedIn);
  const sections = useExploreStore((s) => s.sections);
  const sectionsLoad = useExploreStore((s) => s.sectionsLoad);
  const sectionsError = useExploreStore((s) => s.sectionsError);
  const selected = useExploreStore((s) => s.selected);
  const shelves = useExploreStore((s) => s.shelves);
  const pageLoad = useExploreStore((s) => s.pageLoad);
  const pageError = useExploreStore((s) => s.pageError);
  const ensureCategories = useExploreStore((s) => s.ensureCategories);
  const select = useExploreStore((s) => s.select);
  const clear = useExploreStore((s) => s.clear);
  const opened = useExploreStore((s) => s.opened);
  const close = useExploreStore((s) => s.close);
  const retry = useExploreStore((s) => s.retry);
  const artist = useExploreStore((s) => s.artist);
  const query = useExploreStore((s) => s.query);
  const results = useExploreStore((s) => s.results);
  const searchLoad = useExploreStore((s) => s.searchLoad);
  const searchError = useExploreStore((s) => s.searchError);
  const clearSearch = useExploreStore((s) => s.clearSearch);

  useEffect(() => {
    ensureCategories();
  }, [ensureCategories]);

  const searching = query.length > 0;
  const error = searching ? searchError : selected ? pageError : sectionsError;
  const loading = searching
    ? searchLoad === "loading"
    : selected
      ? pageLoad === "loading"
      : sectionsLoad === "loading";
  // Innermost screen wins: a track list sits on an artist's page, which sits on
  // whatever found the artist.
  const inner = opened ?? artist;

  return (
    <div className="mx-auto flex h-full w-full max-w-5xl flex-col gap-4">
      <PageHeader
        icon={route.icon}
        title={
          inner ? inner.item.title : searching ? `“${query}”` : selected ? selected.title : route.label
        }
        subtitle={
          inner
            ? (inner.item.subtitle ?? "From YouTube Music.")
            : searching
              ? "Results from YouTube Music."
              : selected
                ? "Playlists, albums, songs and videos from YouTube Music."
                : route.tagline
        }
      />

      {signedIn && <SearchBar />}

      {/* One step back, whichever level you're on. */}
      {(selected || inner || searching) && (
        <button
          type="button"
          onClick={inner ? close : searching ? clearSearch : clear}
          className="flex items-center gap-1 self-start text-sm text-text-muted transition-colors hover:text-text"
        >
          <ChevronLeft className="size-4" aria-hidden="true" />
          {inner
            ? opened && artist
              ? artist.item.title
              : (selected?.title ?? (searching ? `“${query}”` : "Back"))
            : searching
              ? "All categories"
              : "All categories"}
        </button>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {sectionsLoad === "ready" && !signedIn ? (
          <Centered
            icon={SquarePlay}
            title="Not signed in to YouTube Music"
            body="Sign in from Settings to browse YouTube's playlists and albums here."
            action={
              <Button variant="primary" onClick={() => setRoute("settings")}>
                <ListMusic className="size-4" aria-hidden="true" />
                Sign in from Settings
              </Button>
            }
          />
        ) : error ? (
          <Centered
            icon={CircleAlert}
            danger
            title={
              searching
                ? "Couldn't search YouTube Music"
                : selected
                  ? "Couldn't load this category"
                  : "Couldn't load Explore"
            }
            body={error}
            action={
              <Button variant="primary" onClick={retry}>
                <RotateCw className="size-4" aria-hidden="true" />
                Retry
              </Button>
            }
          />
        ) : loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-text-muted">
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            {searching
              ? `Searching for “${query}”…`
              : selected
                ? `Loading ${selected.title}…`
                : "Loading Explore…"}
          </div>
        ) : opened ? (
          <OpenedItem />
        ) : artist ? (
          <ShelfList shelves={artist.shelves} />
        ) : searching ? (
          results.length === 0 ? (
            <Centered
              icon={Search}
              title="No results"
              body={`YouTube Music has nothing for “${query}” under this filter. Try another filter, or a different spelling.`}
            />
          ) : (
            <ShelfList shelves={results} />
          )
        ) : selected ? (
          <ShelfList shelves={shelves} />
        ) : (
          <div className="flex flex-col gap-6 pb-4">
            {sections.map((section) => (
              <section key={section.title} className="flex flex-col gap-3">
                <h2 className="text-sm font-medium text-text-muted">{section.title}</h2>
                <div className="flex flex-wrap gap-2">
                  {section.categories.map((c) => (
                    <button
                      key={c.params}
                      type="button"
                      onClick={() => select(c)}
                      className="rounded-control border border-border bg-surface-raised px-4 py-2 text-sm transition-colors hover:border-border-strong hover:bg-surface-overlay"
                    >
                      {c.title}
                    </button>
                  ))}
                </div>
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** One opened playlist/album: what's in it, and where to start. */
function OpenedItem() {
  const opened = useExploreStore((s) => s.opened)!;
  const openError = useExploreStore((s) => s.openError);
  const playOpened = useExploreStore((s) => s.playOpened);
  // Same derivation the Library uses, so a track shows as playing here whether
  // it was started from Explore or anywhere else.
  const current = useEngineStore((s) =>
    s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined,
  );
  const currentId = current?.ytTrack?.videoId ?? null;

  const { item, tracks } = opened;
  const playable = tracks.filter((t) => t.isAvailable).length;

  return (
    <div className="flex flex-col gap-4 pb-4">
      <div className="flex items-center gap-4">
        <Artwork
          art={{ key: item.id, source: "ytmusic", cover: item.thumbnail }}
          seed={item.id}
          label={item.title}
          className="size-28 shrink-0"
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-lg font-semibold">{item.title}</p>
          {item.subtitle && (
            <p className="truncate text-sm text-text-muted">{item.subtitle}</p>
          )}
          <p className="mt-0.5 text-xs text-text-faint">
            {tracks.length} {tracks.length === 1 ? "track" : "tracks"}
            {playable < tracks.length && ` · ${tracks.length - playable} unavailable`}
          </p>
          {playable > 0 && (
            <Button className="mt-3" variant="primary" onClick={() => playOpened(0)}>
              <Play className="size-4" aria-hidden="true" />
              Play all
            </Button>
          )}
        </div>
      </div>

      {openError ? (
        <Centered
          icon={CircleAlert}
          danger
          title="Couldn't open this"
          body={openError}
        />
      ) : (
        <div className="flex flex-col">
          {tracks.map((t, i) => (
            // TrackRow is `h-full` — it takes its height from the row slot the
            // Library's VirtualList gives it. Without one it collapses and the
            // artwork spills into the next row, so supply the same height here.
            <div key={`${t.videoId}:${i}`} style={{ height: TRACK_ROW_H }}>
              <TrackRow
                rank={i + 1}
                title={t.title}
                artist={t.artist}
                durationSecs={t.durationSecs}
                art={{ key: t.videoId, source: "ytmusic", cover: t.thumbnail }}
                seed={t.album ?? t.title}
                source="ytmusic"
                unavailable={!t.isAvailable}
                playing={currentId === t.videoId}
                onPlay={() => playOpened(i)}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** The search box, YouTube's filters, and its completions. */
function SearchBar() {
  const query = useExploreStore((s) => s.query);
  const filter = useExploreStore((s) => s.filter);
  const suggestions = useExploreStore((s) => s.suggestions);
  const search = useExploreStore((s) => s.search);
  const setFilter = useExploreStore((s) => s.setFilter);
  const suggest = useExploreStore((s) => s.suggest);
  const clearSearch = useExploreStore((s) => s.clearSearch);

  // The box holds what's typed; the store holds what was *asked*. Keeping them
  // apart is what lets the field stay editable while results for the previous
  // query are still on screen.
  const [text, setText] = useState(query);
  const [focused, setFocused] = useState(false);
  const debounce = useRef<number | undefined>(undefined);

  // A cleared search elsewhere (Back, a category click) must empty the box too.
  useEffect(() => {
    if (!query) setText("");
  }, [query]);

  useEffect(() => () => window.clearTimeout(debounce.current), []);

  const onType = (v: string) => {
    setText(v);
    // Suggestions chase the keystrokes; the search itself waits for Enter. YT
    // search costs a network round trip per call, and firing one per character
    // would spend six requests to answer a question nobody finished asking.
    window.clearTimeout(debounce.current);
    debounce.current = window.setTimeout(() => suggest(v), 180);
  };

  return (
    <div className="relative flex flex-col gap-2">
      <div className="flex items-center gap-2 rounded-control border border-border bg-surface-raised px-3 focus-within:border-accent">
        <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
        <input
          value={text}
          onChange={(e) => onType(e.target.value)}
          onFocus={() => setFocused(true)}
          // Late enough for a suggestion's click to land first.
          onBlur={() => window.setTimeout(() => setFocused(false), 120)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              setFocused(false);
              search(text);
            }
            if (e.key === "Escape") {
              setText("");
              clearSearch();
            }
          }}
          placeholder="Search songs, videos, artists, albums, playlists…"
          aria-label="Search YouTube Music"
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

      {/* Only once there's a search to filter — they'd do nothing before that. */}
      {query && (
        <div className="flex flex-wrap gap-1.5">
          {SEARCH_FILTERS.map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              aria-pressed={filter === f}
              className={cn(
                "rounded-full border px-3 py-1 text-xs capitalize transition-colors",
                filter === f
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border text-text-muted hover:border-border-strong hover:text-text",
              )}
            >
              {f === "top" ? "Top result" : f}
            </button>
          ))}
        </div>
      )}

      {focused && suggestions.length > 0 && !query && (
        // `bg-surface-overlay`, not `-raised`: this floats over live content with
        // no scrim, so it must be opaque. Under the Dynamic theme `surface-raised`
        // is 55% translucent (glass over the album backdrop) and the For-you
        // chips read straight through it. `surface-overlay` stays solid in every
        // theme — the same token, shadow and ring the Combobox popover uses.
        <ul className="hm-pop absolute left-0 right-0 top-11 z-20 overflow-hidden rounded-control border border-border-strong bg-surface-overlay shadow-2xl ring-1 ring-black/40">
          {suggestions.map((s) => (
            <li key={s}>
              <button
                type="button"
                onMouseDown={() => {
                  setText(s);
                  setFocused(false);
                  search(s);
                }}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-surface-overlay"
              >
                <Search className="size-3.5 shrink-0 text-text-faint" aria-hidden="true" />
                <span className="truncate">{s}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ShelfList({ shelves }: { shelves: ExploreShelf[] }) {
  return (
    <div className="flex flex-col gap-8 pb-4">
      {shelves.map((shelf, i) => (
        // Titles repeat across a page ("Albums" on an artist's page and in its
        // "Fans might also like"), so position is what makes a shelf itself.
        <Shelf key={`${i}:${shelf.title}`} shelf={shelf} />
      ))}
    </div>
  );
}

function Shelf({ shelf }: { shelf: ExploreShelf }) {
  // Songs and videos are rows, everything else is a card — the same split
  // YouTube makes, and for the same reason: a song is a thing you play, and a
  // list of them reads down the page. A card is a place you go.
  const rows = shelf.items.every((i) => i.kind === "song" || i.kind === "video");

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-medium text-text-muted">{shelf.title}</h2>
      {rows ? (
        <div className="flex flex-col">
          {shelf.items.map((item, i) => (
            <Row key={`${item.kind}:${item.id}`} item={item} rank={i + 1} />
          ))}
        </div>
      ) : (
        /* Horizontal, like YouTube's own carousels — a shelf can hold ~100 items
           and stacking them all would bury the next shelf. */
        <div className="flex gap-3 overflow-x-auto pb-2">
          {shelf.items.map((item) => (
            <Tile key={`${item.kind}:${item.id}`} item={item} />
          ))}
        </div>
      )}
    </section>
  );
}

/** One song or video: click plays it. */
function Row({ item, rank }: { item: ExploreItem; rank: number }) {
  const open = useExploreStore((s) => s.open);
  const opening = useExploreStore((s) => s.opening);
  const playingId = useEngineStore((s) => {
    const c = s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined;
    return c?.ytTrack?.videoId ?? null;
  });

  return (
    <div style={{ height: TRACK_ROW_H }}>
      <TrackRow
        rank={rank}
        title={item.title}
        artist={item.artist ?? item.subtitle ?? null}
        durationSecs={item.durationSecs ?? null}
        art={{ key: item.id, source: "ytmusic", cover: item.thumbnail }}
        seed={item.id}
        source="ytmusic"
        playing={playingId === item.id}
        onPlay={() => void open(item)}
        trailing={
          opening === item.id ? (
            <Loader2 className="size-3.5 animate-spin text-text-faint" aria-hidden="true" />
          ) : item.hasVideo ? (
            <VideoIcon className="size-3.5 text-text-faint" aria-hidden="true" />
          ) : undefined
        }
      />
    </div>
  );
}

const TILE_ICON = {
  album: Disc3,
  playlist: ListMusic,
  artist: Mic2,
  song: Play,
  video: VideoIcon,
} as const;

function Tile({ item }: { item: ExploreItem }) {
  const open = useExploreStore((s) => s.open);
  const opening = useExploreStore((s) => s.opening);
  const busy = opening === item.id;
  const Icon = TILE_ICON[item.kind];
  const isArtist = item.kind === "artist";

  return (
    <button
      type="button"
      disabled={busy}
      onClick={() => void open(item)}
      title={item.subtitle ? `${item.title} — ${item.subtitle}` : item.title}
      className={cn(
        "group flex w-40 shrink-0 flex-col gap-2 rounded-lg p-2 text-left transition-colors",
        "hover:bg-surface-raised disabled:opacity-60",
      )}
    >
      <div className="relative">
        <Artwork
          art={{ key: item.id, source: "ytmusic", cover: item.thumbnail }}
          seed={item.id}
          label={item.title}
          // A person is round and a record is square, on YouTube and everywhere
          // else — the shape says which one you're looking at before the label.
          className={cn("aspect-square w-full", isArtist && "rounded-full")}
        />
        <span className="absolute bottom-1 right-1 grid size-6 place-items-center rounded-full bg-surface-overlay/90 opacity-0 transition-opacity group-hover:opacity-100">
          {busy ? (
            <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <Icon className="size-3.5" aria-hidden="true" />
          )}
        </span>
      </div>
      <div className={cn("min-w-0", isArtist && "text-center")}>
        <p className="truncate text-sm font-medium">{item.title}</p>
        {item.subtitle && (
          <p className="truncate text-xs text-text-muted">{item.subtitle}</p>
        )}
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
  action?: React.ReactNode;
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
        <Icon
          className={cn("size-7", danger ? "text-danger" : "text-text-faint")}
          aria-hidden="true"
        />
      </div>
      <div>
        <p className="text-base font-medium">{title}</p>
        <p className="mt-1 max-w-sm text-sm text-text-muted">{body}</p>
      </div>
      {action}
    </div>
  );
}
