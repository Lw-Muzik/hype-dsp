import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Check,
  ChevronDown,
  Cloud,
  ListMusic,
  Music2,
  Plus,
  Search,
  Smartphone,
  Trash2,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { DevicesView } from "@/features/devices/DevicesView";
import { CloudView } from "@/features/cloud/CloudView";
import { AlbumDeck } from "@/features/player/AlbumDeck";
import type { DeckItem } from "@/features/player/AlbumDeck";
import { CategoryChips } from "@/features/player/CategoryChips";
import { AlbumCarousel } from "@/features/player/AlbumCarousel";
import type { CarouselAlbum } from "@/features/player/AlbumCarousel";
import { TrackRow } from "@/features/player/TrackRow";
import { VirtualList } from "@/components/VirtualList";
import { useEngineStore } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import {
  libraryList,
  playlistAdd,
  playlistCreate,
  playlistDelete,
  playlistList,
  playlistRemove,
  playlistTracks,
} from "@/lib/ipc";
import type { LibraryTrack, Playlist } from "@/lib/types";
import { cn } from "@/lib/cn";

interface Album {
  key: string;
  name: string;
  artist: string;
  tracks: LibraryTrack[];
  /** Index of the album's first track within the full list. */
  firstIndex: number;
}

/** Group a track list into albums, preserving first-seen order. */
function groupAlbums(tracks: LibraryTrack[]): Album[] {
  const map = new Map<string, Album>();
  tracks.forEach((t, i) => {
    const name = t.album?.trim() || "Singles";
    const key = name.toLowerCase();
    const existing = map.get(key);
    if (existing) {
      existing.tracks.push(t);
    } else {
      map.set(key, {
        key,
        name,
        artist: t.artist?.trim() || "Unknown artist",
        tracks: [t],
        firstIndex: i,
      });
    }
  });
  return [...map.values()];
}

/** Featured deck items: real albums when there are a few, else the first tracks. */
function pickDeck(albums: Album[], tracks: LibraryTrack[]): DeckItem[] {
  const realAlbums = albums.filter((a) => a.key !== "singles");
  if (realAlbums.length >= 2) {
    return realAlbums.slice(0, 6).map((a) => ({
      key: `a:${a.key}`,
      title: a.name,
      artist: a.artist,
      path: a.tracks[0]?.path ?? null,
      seed: a.name,
      index: a.firstIndex,
    }));
  }
  return tracks.slice(0, 6).map((t, i) => ({
    key: `t:${t.path}`,
    title: t.title,
    artist: t.artist?.trim() || "Unknown artist",
    path: t.path,
    seed: t.album?.trim() || t.title,
    index: i,
  }));
}

/** All albums as carousel cards. */
function toCarousel(albums: Album[]): CarouselAlbum[] {
  return albums.map((a) => ({
    key: a.key,
    name: a.name,
    artist: a.artist,
    path: a.tracks[0]?.path ?? null,
    seed: a.name,
    index: a.firstIndex,
  }));
}

/** Per-row "add to playlist" popover. */
function AddToPlaylist({
  playlists,
  onAdd,
}: {
  playlists: Playlist[];
  onAdd: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", h);
    return () => document.removeEventListener("mousedown", h);
  }, [open]);

  if (playlists.length === 0) return null;
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Add to playlist"
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        className="flex size-7 items-center justify-center rounded-control text-text-faint hover:bg-surface-overlay hover:text-text"
      >
        <Plus className="size-4" aria-hidden="true" />
      </button>
      {open && (
        <div className="absolute right-0 z-20 mt-1 w-44 rounded-control border border-border bg-surface-raised py-1 shadow-lg">
          {playlists.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onAdd(p.id);
                setOpen(false);
              }}
              className="block w-full truncate px-3 py-1.5 text-left text-sm hover:bg-surface-overlay"
            >
              {p.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Collection switcher (Library + playlists) + create. */
function CollectionMenu({
  collectionName,
  playlists,
  collection,
  onSelect,
  onDelete,
  onCreate,
}: {
  collectionName: string;
  playlists: Playlist[];
  collection: string | null;
  onSelect: (id: string | null) => void;
  onDelete: (id: string) => void;
  onCreate: (name: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
        setCreating(false);
      }
    };
    document.addEventListener("mousedown", h);
    return () => document.removeEventListener("mousedown", h);
  }, [open]);

  const submit = () => {
    const n = name.trim();
    setName("");
    setCreating(false);
    setOpen(false);
    if (n) onCreate(n);
  };

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1.5 rounded-control border border-border px-3 py-1.5 text-sm font-medium text-text transition-colors hover:border-border-strong"
      >
        {collection ? (
          <ListMusic className="size-4 text-text-muted" aria-hidden="true" />
        ) : (
          <Music2 className="size-4 text-text-muted" aria-hidden="true" />
        )}
        <span className="max-w-40 truncate">{collectionName}</span>
        <ChevronDown className="size-4 text-text-faint" aria-hidden="true" />
      </button>
      {open && (
        <div className="absolute left-0 z-20 mt-1 w-56 rounded-control border border-border bg-surface-raised py-1 shadow-lg">
          <CollectionRow
            icon={Music2}
            label="Library"
            active={collection === null}
            onClick={() => {
              onSelect(null);
              setOpen(false);
            }}
          />
          {playlists.length > 0 && (
            <div className="my-1 border-t border-border" aria-hidden="true" />
          )}
          {playlists.map((p) => (
            <CollectionRow
              key={p.id}
              icon={ListMusic}
              label={p.name}
              active={collection === p.id}
              onClick={() => {
                onSelect(p.id);
                setOpen(false);
              }}
              onDelete={() => onDelete(p.id)}
            />
          ))}
          <div className="my-1 border-t border-border" aria-hidden="true" />
          {creating ? (
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={submit}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit();
                if (e.key === "Escape") {
                  setName("");
                  setCreating(false);
                }
              }}
              placeholder="Playlist name"
              className="mx-2 my-1 w-[calc(100%-1rem)] rounded-control border border-accent/40 bg-surface px-2.5 py-1.5 text-sm outline-none placeholder:text-text-faint"
            />
          ) : (
            <button
              type="button"
              onClick={() => setCreating(true)}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-text-muted hover:bg-surface-overlay hover:text-text"
            >
              <Plus className="size-4" aria-hidden="true" />
              New playlist
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function CollectionRow({
  icon: Icon,
  label,
  active,
  onClick,
  onDelete,
}: {
  icon: LucideIcon;
  label: string;
  active: boolean;
  onClick: () => void;
  onDelete?: () => void;
}) {
  return (
    <div
      className={cn(
        "group/row flex items-center",
        active ? "text-accent-strong" : "text-text",
      )}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-surface-overlay"
      >
        <Icon className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
        <span className="truncate">{label}</span>
        {active && <Check className="ml-auto size-3.5 shrink-0" aria-hidden="true" />}
      </button>
      {onDelete && (
        <button
          type="button"
          aria-label={`Delete playlist ${label}`}
          onClick={onDelete}
          className="mr-1 hidden size-7 items-center justify-center rounded-control text-text-faint hover:text-danger group-hover/row:flex"
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

/** The Library source: hero deck, genre filter, album strip, and song list. */
function LibraryPanel() {
  const playFromList = useEngineStore((s) => s.playFromList);
  const queue = useEngineStore((s) => s.queue);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  const current = queueIndex >= 0 ? queue[queueIndex] : undefined;
  const playingPath = current?.source === "local" ? (current.id ?? null) : null;
  const libraryVersion = useLibraryStore((s) => s.version);

  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [collection, setCollection] = useState<string | null>(null); // null = Library
  const [tracks, setTracks] = useState<LibraryTrack[]>([]);
  const [category, setCategory] = useState("All");
  const [query, setQuery] = useState("");
  // Defer the (potentially 100k-row) filter so typing stays responsive.
  const deferredQuery = useDeferredValue(query);
  // The single scroll container the song list virtualizes against.
  const scrollRef = useRef<HTMLDivElement>(null);

  const refreshPlaylists = useCallback(() => {
    playlistList().then(setPlaylists).catch(() => {});
  }, []);

  const refreshTracks = useCallback(() => {
    const loader = collection ? playlistTracks(collection) : libraryList();
    loader.then(setTracks).catch(() => setTracks([]));
  }, [collection]);

  useEffect(() => {
    refreshPlaylists();
  }, [refreshPlaylists]);
  // Reload when the collection changes or the library is rescanned (in Settings).
  useEffect(() => {
    refreshTracks();
  }, [refreshTracks, libraryVersion]);
  // A genre filter from another collection may not exist here — reset it.
  useEffect(() => {
    setCategory("All");
  }, [collection]);

  const handleCreate = (name: string) => {
    playlistCreate(name)
      .then((pl) => {
        refreshPlaylists();
        setCollection(pl.id);
      })
      .catch(() => {});
  };

  const handleDeletePlaylist = (id: string) => {
    playlistDelete(id)
      .then(() => {
        refreshPlaylists();
        if (collection === id) setCollection(null);
      })
      .catch(() => {});
  };

  const albums = useMemo(() => groupAlbums(tracks), [tracks]);
  const deck = useMemo(() => pickDeck(albums, tracks), [albums, tracks]);
  const carousel = useMemo(() => toCarousel(albums), [albums]);
  const genres = useMemo(() => {
    const set = new Set<string>();
    for (const t of tracks) {
      const g = t.genre?.trim();
      if (g) set.add(g);
    }
    return ["All", ...[...set].sort((a, b) => a.localeCompare(b))];
  }, [tracks]);

  const visibleTracks = useMemo(() => {
    const q = deferredQuery.trim().toLowerCase();
    if (category === "All" && !q) return tracks;
    return tracks.filter((t) => {
      if (category !== "All" && t.genre?.trim() !== category) return false;
      if (!q) return true;
      return (
        t.title.toLowerCase().includes(q) ||
        (t.artist?.toLowerCase().includes(q) ?? false) ||
        (t.album?.toLowerCase().includes(q) ?? false)
      );
    });
  }, [tracks, category, deferredQuery]);

  const collectionName = collection
    ? (playlists.find((p) => p.id === collection)?.name ?? "Playlist")
    : "Library";

  if (tracks.length === 0 && collection === null) {
    return <EmptyLibrary />;
  }

  return (
    <div
      ref={scrollRef}
      className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto pb-2"
    >
      {deck.length > 0 && (
        <AlbumDeck items={deck} onPlay={(i) => playFromList(tracks, i)} />
      )}

      <CategoryChips
        categories={genres}
        active={category}
        onSelect={setCategory}
      />

      {carousel.length > 1 && (
        <AlbumCarousel
          title="Albums"
          albums={carousel.slice(0, 24)}
          onPlay={(i) => playFromList(tracks, i)}
        />
      )}

      {/* Songs */}
      <section className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <h3 className="text-sm font-semibold">Songs</h3>
            <CollectionMenu
              collectionName={collectionName}
              playlists={playlists}
              collection={collection}
              onSelect={setCollection}
              onDelete={handleDeletePlaylist}
              onCreate={handleCreate}
            />
          </div>
          <div className="relative">
            <Search
              className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-text-faint"
              aria-hidden="true"
            />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search songs"
              aria-label="Search songs"
              className="w-48 rounded-control border border-border bg-surface-raised py-1.5 pl-8 pr-3 text-sm outline-none transition-colors focus:border-border-strong placeholder:text-text-faint"
            />
          </div>
        </div>

        {visibleTracks.length === 0 ? (
          <p className="px-2 py-8 text-center text-sm text-text-muted">
            {collection && tracks.length === 0
              ? "This playlist is empty. Add songs from your library."
              : "No songs match."}
          </p>
        ) : (
          <VirtualList
            items={visibleTracks}
            rowHeight={56}
            scrollRef={scrollRef}
            ariaLabel="Songs"
            getKey={(t) => t.path}
            renderRow={(t, i) => (
              <TrackRow
                track={t}
                rank={i + 1}
                playing={t.path === playingPath}
                onPlay={() => playFromList(visibleTracks, i)}
                trailing={
                  collection ? (
                    <button
                      type="button"
                      aria-label="Remove from playlist"
                      onClick={() =>
                        playlistRemove(collection, t.path)
                          .then(refreshTracks)
                          .catch(() => {})
                      }
                      className="flex size-7 items-center justify-center rounded-control text-text-faint hover:bg-surface hover:text-danger"
                    >
                      <X className="size-4" aria-hidden="true" />
                    </button>
                  ) : (
                    <AddToPlaylist
                      playlists={playlists}
                      onAdd={(id) => void playlistAdd(id, t.path).catch(() => {})}
                    />
                  )
                }
              />
            )}
          />
        )}
      </section>
    </div>
  );
}

/** Shown when the library has never been scanned. */
function EmptyLibrary() {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-surface-raised ring-1 ring-border">
        <Music2 className="size-7 text-text-faint" aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">Your library is empty</p>
        <p className="mt-1 max-w-xs text-sm text-text-muted">
          Add a music folder in Settings to fill your library with its tags and
          cover art.
        </p>
      </div>
      <p className="text-xs text-text-faint">Settings → Music library → Add folder</p>
    </div>
  );
}

/** Source switcher (Library / Phone / Cloud) for the Player hub. */
const SOURCES: { id: PlayerSource; label: string; icon: LucideIcon }[] = [
  { id: "library", label: "Library", icon: Music2 },
  { id: "phone", label: "Phone", icon: Smartphone },
  { id: "cloud", label: "Cloud", icon: Cloud },
];

type PlayerSource = "library" | "phone" | "cloud";

/**
 * The unified music hub: one Player home with a Library / Phone / Cloud source
 * switcher, each rendered with the same rich UI and the shared now-playing bar.
 */
export function PlayerView() {
  const route = routeById("player");
  const [source, setSource] = useState<PlayerSource>("library");

  return (
    <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="flex items-center gap-1 self-start rounded-control border border-border bg-surface-raised p-1">
        {SOURCES.map((s) => {
          const Icon = s.icon;
          const active = source === s.id;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => setSource(s.id)}
              className={cn(
                "flex items-center gap-2 rounded-[7px] px-3.5 py-1.5 text-sm font-medium transition-colors",
                active
                  ? "bg-accent text-surface"
                  : "text-text-muted hover:text-text",
              )}
            >
              <Icon className="size-4" aria-hidden="true" />
              {s.label}
            </button>
          );
        })}
      </div>

      {source === "library" ? (
        <LibraryPanel />
      ) : source === "phone" ? (
        <DevicesView embedded />
      ) : (
        <CloudView embedded />
      )}
    </div>
  );
}
