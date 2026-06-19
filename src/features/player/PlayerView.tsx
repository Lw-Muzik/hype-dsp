import { useCallback, useEffect, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Cloud,
  FolderPlus,
  ListMusic,
  Music2,
  Play,
  Plus,
  Smartphone,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { DevicesView } from "@/features/devices/DevicesView";
import { CloudView } from "@/features/cloud/CloudView";
import { useEngineStore } from "@/stores/engine";
import {
  libraryList,
  libraryScan,
  pickFolder,
  playlistAdd,
  playlistCreate,
  playlistDelete,
  playlistList,
  playlistRemove,
  playlistTracks,
} from "@/lib/ipc";
import type { LibraryTrack, Playlist } from "@/lib/types";
import { formatTime } from "@/lib/format";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";

/** A square gradient cover (with initials) for a track/album. */
function Cover({
  seed,
  label,
  className,
}: {
  seed: string;
  label: string;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "grid shrink-0 place-items-center overflow-hidden font-semibold text-white/90",
        className,
      )}
      style={{ background: coverGradient(seed) }}
      aria-hidden="true"
    >
      <span className="opacity-80">{coverInitials(label)}</span>
    </div>
  );
}

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

/** A spotlighted album/track for the hero carousel. */
interface Featured {
  key: string;
  title: string;
  artist: string;
  subtitle: string;
  seed: string;
  /** Index into the full track list to start playback from. */
  index: number;
}

/**
 * Pick what to spotlight: real albums when the library has a few, otherwise
 * the first handful of tracks (so a flat, tag-less library still gets a hero).
 */
function pickFeatured(albums: Album[], tracks: LibraryTrack[]): Featured[] {
  const realAlbums = albums.filter((a) => a.key !== "singles");
  if (realAlbums.length >= 2) {
    return realAlbums.slice(0, 6).map((a) => ({
      key: `a:${a.key}`,
      title: a.name,
      artist: a.artist,
      subtitle: `${a.tracks.length} track${a.tracks.length === 1 ? "" : "s"}`,
      seed: a.name,
      index: a.firstIndex,
    }));
  }
  return tracks.slice(0, 6).map((t, i) => ({
    key: `t:${t.path}`,
    title: t.title,
    artist: t.artist?.trim() || "Unknown artist",
    subtitle: t.album?.trim() || "Single",
    seed: t.album?.trim() || t.title,
    index: i,
  }));
}

/** Big auto-rotating spotlight at the top of the Library. */
function FeaturedHero({
  items,
  onPlay,
}: {
  items: Featured[];
  onPlay: (f: Featured) => void;
}) {
  const [active, setActive] = useState(0);
  const [paused, setPaused] = useState(false);
  const count = items.length;
  const reduceMotion =
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Keep the active index valid as the library changes underneath us.
  useEffect(() => {
    setActive((a) => (a < count ? a : 0));
  }, [count]);

  // Gently auto-advance — unless hovered, reduced-motion, or a single item.
  useEffect(() => {
    if (paused || reduceMotion || count <= 1) return;
    const id = setInterval(() => setActive((a) => (a + 1) % count), 6000);
    return () => clearInterval(id);
  }, [paused, reduceMotion, count]);

  if (count === 0) return null;
  const f = items[active] ?? items[0]!;

  return (
    <section
      className="px-4 pt-4"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <div className="group/hero relative h-44 overflow-hidden rounded-card">
        {/* Cross-fading gradient backdrops */}
        {items.map((it, i) => (
          <div
            key={it.key}
            className={cn(
              "absolute inset-0 transition-opacity duration-700",
              i === active ? "opacity-100" : "opacity-0",
            )}
            style={{ background: coverGradient(it.seed) }}
            aria-hidden="true"
          />
        ))}
        <div
          className="absolute inset-0 bg-gradient-to-r from-black/75 via-black/35 to-transparent"
          aria-hidden="true"
        />
        <span
          className="pointer-events-none absolute -right-3 top-1/2 -translate-y-1/2 select-none text-[8rem] font-bold leading-none text-white/10"
          aria-hidden="true"
        >
          {coverInitials(f.seed)}
        </span>

        {/* Content */}
        <div className="relative flex h-full flex-col justify-between p-5">
          <span className="inline-flex w-fit items-center gap-1.5 rounded-full bg-white/15 px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wider text-white backdrop-blur">
            <Sparkles className="size-3" aria-hidden="true" />
            Featured
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-2xl font-bold text-white">{f.title}</h2>
            <p className="mt-0.5 truncate text-sm text-white/80">
              {f.artist} · {f.subtitle}
            </p>
            <button
              type="button"
              onClick={() => onPlay(f)}
              className="mt-3 inline-flex items-center gap-2 rounded-full bg-accent px-4 py-2 text-sm font-semibold text-surface shadow-lg transition-transform hover:scale-105 active:scale-100"
            >
              <Play className="size-4 fill-current" aria-hidden="true" />
              Play
            </button>
          </div>
        </div>

        {/* Prev / next (revealed on hover) */}
        {count > 1 && (
          <>
            <button
              type="button"
              aria-label="Previous featured"
              onClick={() => setActive((a) => (a - 1 + count) % count)}
              className="absolute left-2 top-1/2 grid size-8 -translate-y-1/2 place-items-center rounded-full bg-black/40 text-white opacity-0 transition-opacity hover:bg-black/60 group-hover/hero:opacity-100"
            >
              <ChevronLeft className="size-5" aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="Next featured"
              onClick={() => setActive((a) => (a + 1) % count)}
              className="absolute right-2 top-1/2 grid size-8 -translate-y-1/2 place-items-center rounded-full bg-black/40 text-white opacity-0 transition-opacity hover:bg-black/60 group-hover/hero:opacity-100"
            >
              <ChevronRight className="size-5" aria-hidden="true" />
            </button>
          </>
        )}
      </div>

      {/* Dots */}
      {count > 1 && (
        <div className="mt-2 flex justify-center gap-1.5">
          {items.map((it, i) => (
            <button
              key={it.key}
              type="button"
              aria-label={`Show featured item ${i + 1}`}
              aria-current={i === active}
              onClick={() => setActive(i)}
              className={cn(
                "h-1.5 rounded-full transition-all",
                i === active
                  ? "w-5 bg-accent"
                  : "w-1.5 bg-border-strong hover:bg-text-faint",
              )}
            />
          ))}
        </div>
      )}
    </section>
  );
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

/** The Library source: local scanned library + playlists, rich UI. */
function LibraryPanel() {
  const playFromList = useEngineStore((s) => s.playFromList);
  const queue = useEngineStore((s) => s.queue);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  // Local queue items use the file path as their id, so this matches by path.
  const current = queueIndex >= 0 ? queue[queueIndex] : undefined;
  const playingPath =
    current?.source === "local" ? (current.id ?? null) : null;

  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [collection, setCollection] = useState<string | null>(null); // null = Library
  const [tracks, setTracks] = useState<LibraryTrack[]>([]);
  const [scanning, setScanning] = useState(false);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  const refreshPlaylists = useCallback(() => {
    playlistList()
      .then(setPlaylists)
      .catch(() => {});
  }, []);

  const refreshTracks = useCallback(() => {
    const loader = collection ? playlistTracks(collection) : libraryList();
    loader.then(setTracks).catch(() => setTracks([]));
  }, [collection]);

  useEffect(() => {
    refreshPlaylists();
  }, [refreshPlaylists]);
  useEffect(() => {
    refreshTracks();
  }, [refreshTracks]);

  const handleScan = async () => {
    const dir = await pickFolder();
    if (!dir) return;
    setScanning(true);
    try {
      await libraryScan(dir);
      setCollection(null);
      refreshTracks();
    } finally {
      setScanning(false);
    }
  };

  const handleCreate = () => {
    const name = newName.trim();
    setNewName("");
    setCreating(false);
    if (!name) return;
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

  const activeName = collection
    ? (playlists.find((p) => p.id === collection)?.name ?? "Playlist")
    : "Library";

  // Albums strip + featured hero only on the Library (not inside a playlist).
  const albums = collection === null ? groupAlbums(tracks) : [];
  const featured = collection === null ? pickFeatured(albums, tracks) : [];

  return (
    <div className="flex min-h-0 flex-1 gap-4">
        {/* Collections sidebar */}
        <aside className="flex w-52 shrink-0 flex-col gap-1">
          <CollectionItem
            icon={Music2}
            label="Library"
            active={collection === null}
            onClick={() => setCollection(null)}
          />
          <div className="mt-2 px-2 text-[11px] font-medium uppercase tracking-wide text-text-faint">
            Playlists
          </div>
          {playlists.map((p) => (
            <CollectionItem
              key={p.id}
              icon={ListMusic}
              label={p.name}
              active={collection === p.id}
              onClick={() => setCollection(p.id)}
              onDelete={() => handleDeletePlaylist(p.id)}
            />
          ))}
          {creating ? (
            <input
              autoFocus
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onBlur={handleCreate}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate();
                if (e.key === "Escape") {
                  setNewName("");
                  setCreating(false);
                }
              }}
              placeholder="Playlist name"
              className="mx-1 rounded-control border border-accent/40 bg-surface px-2.5 py-1.5 text-sm outline-none placeholder:text-text-faint"
            />
          ) : (
            <button
              type="button"
              onClick={() => setCreating(true)}
              className="mx-1 mt-1 flex items-center gap-2 rounded-control px-2.5 py-1.5 text-sm text-text-muted hover:bg-surface-raised hover:text-text"
            >
              <Plus className="size-4" aria-hidden="true" />
              New playlist
            </button>
          )}
        </aside>

        {/* Track table */}
        <div className="flex min-w-0 flex-1 flex-col rounded-card border border-border bg-surface-raised">
          <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
            <div className="min-w-0">
              <h3 className="truncate text-sm font-medium">{activeName}</h3>
              <p className="text-xs text-text-muted">{tracks.length} tracks</p>
            </div>
            {collection === null && (
              <Button variant="secondary" onClick={handleScan} disabled={scanning}>
                <FolderPlus className="size-4" aria-hidden="true" />
                {scanning ? "Scanning…" : "Scan folder"}
              </Button>
            )}
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {tracks.length === 0 ? (
              <div className="flex h-full min-h-[200px] flex-col items-center justify-center gap-2 p-8 text-center">
                <Music2 className="size-8 text-text-faint" aria-hidden="true" />
                <p className="text-sm text-text-muted">
                  {collection === null
                    ? "Your library is empty. Scan a folder to add music."
                    : "This playlist is empty. Add tracks from your library."}
                </p>
              </div>
            ) : (
              <div className="flex flex-col">
                {/* Featured spotlight (Library) */}
                {featured.length > 0 && (
                  <FeaturedHero
                    items={featured}
                    onPlay={(f) => playFromList(tracks, f.index)}
                  />
                )}

                {/* Albums strip (Library) */}
                {albums.length > 1 && (
                  <section className="border-b border-border px-4 pb-4 pt-3">
                    <h4 className="mb-2 text-xs font-medium uppercase tracking-wider text-text-faint">
                      Albums
                    </h4>
                    <div className="flex gap-5 overflow-x-auto pb-1">
                      {albums.map((a) => (
                        <button
                          key={a.key}
                          type="button"
                          onClick={() => playFromList(tracks, a.firstIndex)}
                          className="group flex w-24 shrink-0 flex-col items-center gap-2 text-center"
                          title={`${a.name} — ${a.artist}`}
                        >
                          <Cover
                            seed={a.name}
                            label={a.name}
                            className="size-24 rounded-full text-lg shadow-md ring-1 ring-white/10 transition-transform group-hover:scale-105"
                          />
                          <span className="w-full truncate text-xs font-medium">
                            {a.name}
                          </span>
                          <span className="-mt-1.5 w-full truncate text-[11px] text-text-faint">
                            {a.artist}
                          </span>
                        </button>
                      ))}
                    </div>
                  </section>
                )}

                {/* Track list — ranked, with cover art */}
                <div className="hidden items-center gap-3 px-4 pb-1 pt-3 text-[11px] font-medium uppercase tracking-wider text-text-faint sm:flex">
                  <span className="w-6 text-right">#</span>
                  <span className="w-11" />
                  <span className="flex-1">Song</span>
                  <span className="w-16 text-right">Time</span>
                  <span className="w-7" />
                </div>
                <ol className="flex flex-col px-2 pb-2">
                  {tracks.map((t, i) => {
                    const isPlaying = t.path === playingPath;
                    return (
                      <li
                        key={t.path}
                        onClick={() => playFromList(tracks, i)}
                        className={cn(
                          "group flex cursor-pointer items-center gap-3 rounded-control px-2 py-1.5 transition-colors hover:bg-surface-overlay",
                          isPlaying && "bg-accent-muted/40",
                        )}
                      >
                        <span
                          className={cn(
                            "w-6 text-right text-xs tabular-nums",
                            isPlaying ? "text-accent-strong" : "text-text-faint",
                          )}
                        >
                          {String(i + 1).padStart(2, "0")}
                        </span>
                        <div className="relative">
                          <Cover
                            seed={t.album?.trim() || t.title}
                            label={t.title}
                            className="size-11 rounded-md text-sm"
                          />
                          <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
                            <Play className="size-4 text-white" aria-hidden="true" />
                          </span>
                        </div>
                        <div className="min-w-0 flex-1">
                          <p
                            className={cn(
                              "truncate text-sm font-medium",
                              isPlaying && "text-accent-strong",
                            )}
                          >
                            {t.title}
                          </p>
                          <p className="truncate text-xs text-text-muted">
                            {t.artist ?? "—"}
                          </p>
                        </div>
                        <span className="w-16 shrink-0 text-right text-xs tabular-nums text-text-muted">
                          {formatTime(t.durationSecs)}
                        </span>
                        <div onClick={(e) => e.stopPropagation()}>
                          {collection ? (
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
                              onAdd={(id) =>
                                void playlistAdd(id, t.path).catch(() => {})
                              }
                            />
                          )}
                        </div>
                      </li>
                    );
                  })}
                </ol>
              </div>
            )}
          </div>
        </div>
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

function CollectionItem({
  icon: Icon,
  label,
  active,
  onClick,
  onDelete,
}: {
  icon: typeof Music2;
  label: string;
  active: boolean;
  onClick: () => void;
  onDelete?: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center rounded-control transition-colors",
        active ? "bg-surface-overlay text-text" : "text-text-muted hover:bg-surface-raised",
      )}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-sm"
      >
        <Icon className="size-4 shrink-0" aria-hidden="true" />
        <span className="truncate">{label}</span>
      </button>
      {onDelete && (
        <button
          type="button"
          aria-label={`Delete playlist ${label}`}
          onClick={onDelete}
          className="mr-1 hidden size-7 items-center justify-center rounded-control text-text-faint hover:text-danger group-hover:flex"
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}
