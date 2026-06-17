import { useCallback, useEffect, useRef, useState } from "react";
import {
  FolderPlus,
  ListMusic,
  Music2,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { TransportBar } from "@/features/player/TransportBar";
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
import { cn } from "@/lib/cn";

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

export function PlayerView() {
  const route = routeById("player");
  const playFromList = useEngineStore((s) => s.playFromList);
  const queue = useEngineStore((s) => s.queue);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  const playingPath =
    queueIndex >= 0 ? (queue[queueIndex]?.path ?? null) : null;

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

  return (
    <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <TransportBar />

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
              <table className="w-full text-sm">
                <thead className="sticky top-0 bg-surface-raised text-left text-xs text-text-faint">
                  <tr className="border-b border-border">
                    <th className="w-10 py-2 pl-4 font-medium">#</th>
                    <th className="py-2 font-medium">Title</th>
                    <th className="hidden py-2 font-medium md:table-cell">Artist</th>
                    <th className="w-16 py-2 pr-2 text-right font-medium">Time</th>
                    <th className="w-12 py-2 pr-4" />
                  </tr>
                </thead>
                <tbody>
                  {tracks.map((t, i) => {
                    const isPlaying = t.path === playingPath;
                    return (
                      <tr
                        key={t.path}
                        onClick={() => playFromList(tracks, i)}
                        className={cn(
                          "cursor-pointer border-b border-border/60 transition-colors hover:bg-surface-overlay",
                          isPlaying && "bg-accent-muted/40",
                        )}
                      >
                        <td className="py-2 pl-4 text-text-faint tabular-nums">
                          {isPlaying ? (
                            <span className="text-accent-strong">▶</span>
                          ) : (
                            i + 1
                          )}
                        </td>
                        <td className="min-w-0 py-2">
                          <span
                            className={cn(
                              "block truncate",
                              isPlaying && "text-accent-strong",
                            )}
                          >
                            {t.title}
                          </span>
                        </td>
                        <td className="hidden truncate py-2 text-text-muted md:table-cell">
                          {t.artist ?? "—"}
                        </td>
                        <td className="py-2 pr-2 text-right tabular-nums text-text-muted">
                          {formatTime(t.durationSecs)}
                        </td>
                        <td className="py-2 pr-4">
                          <div
                            className="flex justify-end"
                            onClick={(e) => e.stopPropagation()}
                          >
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
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        </div>
      </div>
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
