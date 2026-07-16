import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  CircleAlert,
  Cloud,
  Disc3,
  FolderOpen,
  LayoutGrid,
  List,
  ListMusic,
  Loader2,
  Music2,
  Play,
  RotateCw,
  Search,
  Smartphone,
  SquarePlay,
  Tag,
  Users,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import { trackArt, useMusicLibrary } from "@/features/player/useMusicLibrary";
import type { MusicTrack } from "@/features/player/useMusicLibrary";
import type { ArtSource } from "@/lib/useTrackArtwork";
import { AlbumDeck } from "@/features/player/AlbumDeck";
import type { DeckItem } from "@/features/player/AlbumDeck";
import { TrackRow, TRACK_ROW_H } from "@/features/player/TrackRow";
import { DownloadAction } from "@/features/ytmusic/DownloadAction";
import { Artwork } from "@/features/player/Artwork";
import { VirtualList } from "@/components/VirtualList";
import { VirtualGrid } from "@/components/VirtualGrid";
import { Button } from "@/components/Button";
import { cn } from "@/lib/cn";

type SourceFilter = "all" | "local" | "phone" | "cloud" | "ytmusic";
type Facet = "songs" | "albums" | "artists" | "folders" | "genres";
type ViewMode = "list" | "grid";

const FACETS: { id: Facet; label: string; icon: LucideIcon }[] = [
  { id: "songs", label: "Songs", icon: Music2 },
  { id: "albums", label: "Albums", icon: Disc3 },
  { id: "artists", label: "Artists", icon: Users },
  { id: "folders", label: "Folders", icon: FolderOpen },
  { id: "genres", label: "Genres", icon: Tag },
];

interface Group {
  key: string;
  label: string;
  subtitle: string;
  seed: string;
  /** Cover art from the group's first track (for the Albums facet). */
  art: ArtSource;
  tracks: MusicTrack[];
}

/** Group a track set by the active facet, preserving first-seen order. */
function groupTracks(tracks: MusicTrack[], facet: Facet): Group[] {
  const keyer: Record<Exclude<Facet, "songs">, (t: MusicTrack) => string> = {
    albums: (t) => t.album?.trim() || "Singles",
    artists: (t) => t.artist?.trim() || "Unknown artist",
    folders: (t) => t.folder?.trim() || "Other",
    genres: (t) => t.genre?.trim() || "Unknown genre",
  };
  const fn = keyer[facet as Exclude<Facet, "songs">];
  const map = new Map<string, Group>();
  for (const t of tracks) {
    const label = fn(t);
    const existing = map.get(label.toLowerCase());
    if (existing) {
      existing.tracks.push(t);
    } else {
      map.set(label.toLowerCase(), {
        key: label.toLowerCase(),
        label,
        subtitle: "",
        seed: facet === "albums" ? label : `${facet}:${label}`,
        art: trackArt(t),
        tracks: [t],
      });
    }
  }
  const groups = [...map.values()];
  for (const g of groups) {
    g.subtitle =
      facet === "albums"
        ? (g.tracks[0]?.artist ?? "Unknown artist")
        : `${g.tracks.length} song${g.tracks.length === 1 ? "" : "s"}`;
  }
  return groups.sort((a, b) => a.label.localeCompare(b.label));
}

// The hero deck only needs a handful of albums, so it samples a bounded prefix
// rather than grouping the entire library — that keeps it O(1) w.r.t. library
// size (grouping millions of tracks just to pick 6 would block the main thread).
const DECK_SAMPLE = 2000;

/** Featured deck items for the hero (top albums, else first tracks). */
function pickDeck(tracks: MusicTrack[]): DeckItem[] {
  // A prefix is safe: `sample` is a prefix of `tracks`, so an index into
  // `sample` is the same index into `tracks` (used to start playback there).
  const sample = tracks.length > DECK_SAMPLE ? tracks.slice(0, DECK_SAMPLE) : tracks;
  const albums = groupTracks(sample, "albums").filter((g) => g.key !== "singles");
  if (albums.length >= 2) {
    return albums.slice(0, 6).map((a) => ({
      key: `a:${a.key}`,
      title: a.label,
      artist: a.subtitle,
      art: a.art,
      seed: a.seed,
      index: sample.indexOf(a.tracks[0]!),
    }));
  }
  return sample.slice(0, 6).map((t, i) => ({
    key: `t:${t.uid}`,
    title: t.title,
    artist: t.artist ?? "Unknown artist",
    art: trackArt(t),
    seed: t.album?.trim() || t.title,
    index: i,
  }));
}

/** YT Music lists region-blocked / removed tracks so a playlist matches what the
 *  user sees on YouTube, but nothing can play them. Every other source only
 *  lists what it can play. */
function isUnavailable(t: MusicTrack): boolean {
  return t.ytTrack?.isAvailable === false;
}

function matches(t: MusicTrack, q: string): boolean {
  return (
    t.title.toLowerCase().includes(q) ||
    (t.artist?.toLowerCase().includes(q) ?? false) ||
    (t.album?.toLowerCase().includes(q) ?? false) ||
    (t.folder?.toLowerCase().includes(q) ?? false)
  );
}

const ROW_H = TRACK_ROW_H;
const GROUP_H = 64;
// Grid cells: min card width + height reserved for the title/subtitle below art.
const GRID_MIN_COL = 150;
const GRID_TEXT_H = 52;

/**
 * The unified music browser: every source (local library + paired phones +
 * connected cloud) merged into one collection, browsable by Songs / Albums /
 * Artists / Folders / Genres, with a global search across all of them and a
 * source filter. Replaces the old per-source views.
 */
export function MusicLibrary() {
  const {
    tracks,
    localTracks,
    phoneTracks,
    cloudTracks,
    ytmusicTracks,
    library,
    phone,
    cloud,
    ytmusic,
  } = useMusicLibrary();
  const playQueueItems = useEngineStore((s) => s.playQueueItems);
  const current = useEngineStore((s) =>
    s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined,
  );
  const playingKey = current ? `${current.source}:${current.id}` : null;
  const setRoute = useUiStore((s) => s.setRoute);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [facet, setFacet] = useState<Facet>("songs");
  const [view, setView] = useState<ViewMode>("list");
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [drill, setDrill] = useState<{ label: string; tracks: MusicTrack[] } | null>(null);

  // Reset transient view state when the source filter changes.
  useEffect(() => setDrill(null), [sourceFilter, facet]);

  // Pick the active source's list directly — no O(n) `.filter`/spread over the
  // merged library on every render. Each list is already maintained separately
  // in the store; "all" is the pre-merged array.
  const filtered =
    sourceFilter === "all"
      ? tracks
      : sourceFilter === "local"
        ? localTracks
        : sourceFilter === "phone"
          ? phoneTracks
          : sourceFilter === "cloud"
            ? cloudTracks
            : ytmusicTracks;

  // Heavy browse derivations (search / facet grouping / hero deck) run against a
  // *deferred* copy so they never block input or navigation: while a large
  // library streams in (~5 publishes/sec) React keeps the last result on screen
  // and recomputes these at low priority, coalescing bursts instead of
  // re-deriving synchronously on the render critical path for every publish.
  const deferredFiltered = useDeferredValue(filtered);

  const q = deferredQuery.trim().toLowerCase();
  const searching = q.length > 0;

  const searchResults = useMemo(
    () => (searching ? deferredFiltered.filter((t) => matches(t, q)) : []),
    [deferredFiltered, q, searching],
  );

  const groups = useMemo(
    () => (!searching && facet !== "songs" && !drill ? groupTracks(deferredFiltered, facet) : []),
    [deferredFiltered, facet, searching, drill],
  );

  const deck = useMemo(
    () => (!searching && facet === "songs" && !drill ? pickDeck(deferredFiltered) : []),
    [deferredFiltered, facet, searching, drill],
  );

  // The track list currently shown (and what playback enqueues).
  const shownTracks = searching ? searchResults : drill ? drill.tracks : filtered;

  // Unavailable tracks can't stream, so they never enter the queue — otherwise
  // next/prev would walk onto dead entries. The start index is re-based onto the
  // playable set (as `playCloudList` does when filtering folders out), skipping
  // forward when the requested one is itself unavailable — "Play all" and the
  // hero deck can both land on one.
  const playAt = (list: MusicTrack[], index: number) => {
    const from = list.findIndex((t, i) => i >= index && !isUnavailable(t));
    if (from < 0) return;
    const target = list[from]!;
    const playable = list.filter((t) => !isUnavailable(t));
    playQueueItems(
      playable,
      Math.max(
        0,
        playable.findIndex((t) => t.uid === target.uid),
      ),
    );
  };

  // Has the active source finished its first load? For "All" we wait on every
  // source so we don't flash an empty state while others are still arriving.
  const activeReady =
    sourceFilter === "all"
      ? library.ready && phone.ready && cloud.ready && ytmusic.ready
      : sourceFilter === "local"
        ? library.ready
        : sourceFilter === "phone"
          ? phone.ready
          : sourceFilter === "cloud"
            ? cloud.ready
            : ytmusic.ready;

  // The signed-in account's listing failed. Distinct from "not connected": the
  // account is fine, the fetch wasn't, so the fix is Retry — not a trip to
  // Settings to connect something that's already connected.
  const sourceError = sourceFilter === "ytmusic" ? ytmusic.error : null;

  // Phone/Cloud/YouTube Music selected, finished checking, and genuinely not
  // connected → offer to set them up (only *after* the status check, never
  // mid-load).
  const needsConnect =
    !sourceError &&
    ((sourceFilter === "phone" && phone.ready && !phone.connected) ||
      (sourceFilter === "cloud" && cloud.ready && !cloud.connected) ||
      (sourceFilter === "ytmusic" && ytmusic.ready && !ytmusic.connected));

  // First load still running with nothing to show yet → show a loading state
  // instead of a blank pane or a misleading "not connected" prompt.
  const showLoading = !activeReady && !needsConnect && filtered.length === 0;
  const loadingMessage =
    sourceFilter === "phone"
      ? "Loading music from your phone…"
      : sourceFilter === "cloud"
        ? "Loading your cloud library…"
        : sourceFilter === "ytmusic"
          ? "Loading your YouTube Music playlists…"
          : "Loading your music…";

  // A source is still streaming in behind already-visible tracks.
  const stillLoading =
    library.loading || phone.loading || cloud.loading || ytmusic.loading;

  // While a big local library pages in, show how far along we are.
  const loadProgress =
    library.loading && library.total > library.count
      ? ` ${library.count.toLocaleString()} / ${library.total.toLocaleString()}`
      : "";

  return (
    <div ref={scrollRef} className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pb-2">
      {/* Source filter + global search */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-1 rounded-control border border-border bg-surface-raised p-1">
          <SourcePill label="All" active={sourceFilter === "all"} onClick={() => setSourceFilter("all")} />
          <SourcePill
            icon={Music2}
            label="Library"
            count={library.count}
            active={sourceFilter === "local"}
            onClick={() => setSourceFilter("local")}
          />
          <SourcePill
            icon={Smartphone}
            label="Phone"
            count={phone.connected ? phone.count : undefined}
            dot={phone.connected}
            active={sourceFilter === "phone"}
            onClick={() => setSourceFilter("phone")}
          />
          <SourcePill
            icon={Cloud}
            label="Cloud"
            count={cloud.connected ? cloud.count : undefined}
            dot={cloud.connected}
            active={sourceFilter === "cloud"}
            onClick={() => setSourceFilter("cloud")}
          />
          <SourcePill
            icon={SquarePlay}
            label="YouTube"
            count={ytmusic.connected ? ytmusic.count : undefined}
            dot={ytmusic.connected}
            active={sourceFilter === "ytmusic"}
            onClick={() => setSourceFilter("ytmusic")}
          />
        </div>
        <div className="flex items-center gap-2">
          <ViewToggle view={view} onChange={setView} />
          <div className="relative">
            <Search
              className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-text-faint"
              aria-hidden="true"
            />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search everything"
              aria-label="Search all sources"
              className="w-56 rounded-control border border-border bg-surface-raised py-1.5 pl-8 pr-3 text-sm outline-none transition-colors focus:border-border-strong placeholder:text-text-faint"
            />
          </div>
        </div>
      </div>

      {showLoading ? (
        <LibraryLoading message={loadingMessage} />
      ) : sourceError ? (
        <SourceErrorPrompt message={sourceError} onRetry={ytmusic.retry} />
      ) : needsConnect ? (
        <ConnectPrompt
          kind={
            sourceFilter === "phone"
              ? "phone"
              : sourceFilter === "ytmusic"
                ? "ytmusic"
                : "cloud"
          }
          onConnect={() => setRoute("settings")}
        />
      ) : filtered.length === 0 ? (
        <EmptySource source={sourceFilter} />
      ) : (
        <>
          {/* Hero deck (Songs facet only) */}
          {deck.length > 0 && (
            <AlbumDeck
              items={deck}
              onPlay={(i) => {
                // Deck items index into either album-firsts or the first tracks.
                playAt(filtered, Math.max(0, i));
              }}
            />
          )}

          {/* Facet tabs + search/drill header */}
          {searching ? (
            <SectionHeader title={`Results for “${deferredQuery.trim()}”`} subtitle={`${searchResults.length} songs`} />
          ) : drill ? (
            <div className="flex shrink-0 items-center gap-3">
              <button
                type="button"
                onClick={() => setDrill(null)}
                className="flex items-center gap-1 rounded-control border border-border px-2.5 py-1.5 text-sm text-text-muted transition-colors hover:border-border-strong hover:text-text"
              >
                <ChevronLeft className="size-4" aria-hidden="true" />
                Back
              </button>
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold">{drill.label}</p>
                <p className="text-xs text-text-muted">{drill.tracks.length} songs</p>
              </div>
              <Button
                variant="primary"
                onClick={() => playAt(drill.tracks, 0)}
                className="ml-auto"
              >
                <Play className="size-4 fill-current" aria-hidden="true" />
                Play all
              </Button>
            </div>
          ) : (
            <FacetTabs facet={facet} onSelect={setFacet} />
          )}

          {/* Content — re-keyed so it animates in on any view/facet/search change. */}
          <div
            key={`${view}|${searching ? "search" : drill ? "drill" : facet}`}
            className="hm-view-enter min-h-0 shrink-0"
          >
            {searching || drill || facet === "songs" ? (
              shownTracks.length === 0 ? (
                <Empty message={searching ? "No matches." : "Nothing here yet."} />
              ) : view === "grid" ? (
                <VirtualGrid
                  items={shownTracks}
                  minColWidth={GRID_MIN_COL}
                  textHeight={GRID_TEXT_H}
                  scrollRef={scrollRef}
                  ariaLabel="Songs"
                  getKey={(t) => t.uid}
                  renderCell={(t, i) => (
                    <TrackCard
                      track={t}
                      playing={`${t.source}:${t.id}` === playingKey}
                      onPlay={() => playAt(shownTracks, i)}
                    />
                  )}
                />
              ) : (
                <VirtualList
                  items={shownTracks}
                  rowHeight={ROW_H}
                  scrollRef={scrollRef}
                  ariaLabel="Songs"
                  getKey={(t) => t.uid}
                  renderRow={(t, i) => (
                    <TrackRow
                      rank={i + 1}
                      title={t.title}
                      artist={t.artist}
                      durationSecs={t.durationSecs}
                      art={trackArt(t)}
                      seed={t.album?.trim() || t.title}
                      source={t.source}
                      unavailable={isUnavailable(t)}
                      playing={`${t.source}:${t.id}` === playingKey}
                      onPlay={() => playAt(shownTracks, i)}
                      trailing={
                        t.ytTrack && <DownloadAction track={t.ytTrack} />
                      }
                    />
                  )}
                />
              )
            ) : groups.length === 0 ? (
              <Empty message="Nothing here yet." />
            ) : view === "grid" ? (
              <VirtualGrid
                items={groups}
                minColWidth={GRID_MIN_COL}
                textHeight={GRID_TEXT_H}
                scrollRef={scrollRef}
                ariaLabel={facet}
                getKey={(g) => g.key}
                renderCell={(g) => (
                  <GroupCard
                    group={g}
                    facetIcon={FACETS.find((f) => f.id === facet)!.icon}
                    showArt={facet === "albums"}
                    onOpen={() => setDrill({ label: g.label, tracks: g.tracks })}
                  />
                )}
              />
            ) : (
              <VirtualList
                items={groups}
                rowHeight={GROUP_H}
                scrollRef={scrollRef}
                ariaLabel={facet}
                getKey={(g) => g.key}
                renderRow={(g) => (
                  <GroupRow
                    group={g}
                    facetIcon={FACETS.find((f) => f.id === facet)!.icon}
                    showArt={facet === "albums"}
                    onOpen={() => setDrill({ label: g.label, tracks: g.tracks })}
                  />
                )}
              />
            )}
          </div>
        </>
      )}

      {stillLoading && !showLoading && filtered.length > 0 && (
        <p className="flex items-center justify-center gap-2 px-2 text-center text-xs text-text-faint">
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          Loading more music…{loadProgress}
        </p>
      )}
    </div>
  );
}

function SourcePill({
  icon: Icon,
  label,
  count,
  dot,
  active,
  onClick,
}: {
  icon?: LucideIcon;
  label: string;
  count?: number;
  dot?: boolean;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-[7px] px-3 py-1.5 text-sm font-medium transition-colors",
        active ? "bg-accent text-on-accent" : "text-text-muted hover:text-text",
      )}
    >
      {Icon && <Icon className="size-4" aria-hidden="true" />}
      {label}
      {count != null && (
        <span className={cn("tabular-nums text-xs", active ? "text-on-accent" : "text-text-faint")}>
          {count.toLocaleString()}
        </span>
      )}
      {dot === false && <span className="size-1.5 rounded-full bg-text-faint" aria-hidden="true" />}
    </button>
  );
}

function FacetTabs({ facet, onSelect }: { facet: Facet; onSelect: (f: Facet) => void }) {
  return (
    <div
      className="flex shrink-0 gap-2 overflow-x-auto pb-1"
      role="tablist"
      aria-label="Browse by"
    >
      {FACETS.map((f) => {
        const Icon = f.icon;
        const active = f.id === facet;
        return (
          <button
            key={f.id}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onSelect(f.id)}
            className={cn(
              "flex shrink-0 items-center gap-1.5 rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors",
              active
                ? "bg-accent text-on-accent"
                : "border border-border text-text-muted hover:border-border-strong hover:text-text",
            )}
          >
            <Icon className="size-4" aria-hidden="true" />
            {f.label}
          </button>
        );
      })}
    </div>
  );
}

function GroupRow({
  group,
  facetIcon: Icon,
  showArt,
  onOpen,
}: {
  group: Group;
  facetIcon: LucideIcon;
  showArt: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex h-full w-full items-center gap-3 rounded-control px-2 text-left transition-colors hover:bg-surface-overlay"
    >
      {showArt ? (
        <Artwork
          art={group.art}
          seed={group.seed}
          label={group.label}
          rounded="rounded-md"
          className="size-12"
        />
      ) : (
        <span className="grid size-12 shrink-0 place-items-center rounded-md bg-surface-overlay text-text-muted">
          <Icon className="size-5" aria-hidden="true" />
        </span>
      )}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{group.label}</p>
        <p className="truncate text-xs text-text-muted">{group.subtitle}</p>
      </div>
      <span className="grid size-7 place-items-center rounded-full text-text-faint opacity-0 transition-opacity group-hover:opacity-100">
        <Play className="size-4 fill-current" aria-hidden="true" />
      </span>
    </button>
  );
}

function ViewToggle({ view, onChange }: { view: ViewMode; onChange: (v: ViewMode) => void }) {
  const btn = (mode: ViewMode, Icon: LucideIcon, label: string) => (
    <button
      type="button"
      aria-label={label}
      aria-pressed={view === mode}
      title={label}
      onClick={() => onChange(mode)}
      className={cn(
        "grid size-7 place-items-center rounded-[7px] transition-colors",
        view === mode ? "bg-accent text-on-accent" : "text-text-muted hover:text-text",
      )}
    >
      <Icon className="size-4" aria-hidden="true" />
    </button>
  );
  return (
    <div className="flex items-center gap-0.5 rounded-control border border-border bg-surface-raised p-0.5">
      {btn("list", List, "List view")}
      {btn("grid", LayoutGrid, "Grid view")}
    </div>
  );
}

/** The corner badge marking a card's non-local source. */
const CARD_BADGE: Record<
  Exclude<MusicTrack["source"], "local">,
  LucideIcon
> = {
  phone: Smartphone,
  cloud: Cloud,
  ytmusic: SquarePlay,
};

/** A track rendered as a grid card (square cover + title + artist). */
function TrackCard({
  track,
  playing,
  onPlay,
}: {
  track: MusicTrack;
  playing: boolean;
  onPlay: () => void;
}) {
  const unavailable = isUnavailable(track);
  const Badge = track.source === "local" ? null : CARD_BADGE[track.source];
  return (
    <button
      type="button"
      onClick={unavailable ? undefined : onPlay}
      disabled={unavailable}
      title={unavailable ? "Not available on YouTube Music" : undefined}
      className={cn(
        "group flex h-full w-full flex-col gap-2 text-left",
        unavailable && "cursor-not-allowed",
      )}
    >
      <div
        className={cn(
          "relative",
          playing && "rounded-xl ring-2 ring-accent",
          unavailable && "opacity-40 grayscale",
        )}
      >
        <Artwork
          art={trackArt(track)}
          seed={track.album?.trim() || track.title}
          label={track.title}
          rounded="rounded-xl"
          className="aspect-square w-full shadow-md ring-1 ring-white/5"
        />
        {!unavailable && (
          <span className="absolute inset-0 grid place-items-center rounded-xl bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
            <span className="grid size-10 place-items-center rounded-full bg-accent text-surface shadow-lg">
              <Play className="size-5 fill-current" aria-hidden="true" />
            </span>
          </span>
        )}
        {Badge && (
          <span className="absolute right-1.5 top-1.5 grid size-5 place-items-center rounded-full bg-black/55 text-white backdrop-blur">
            <Badge className="size-3" aria-hidden="true" />
          </span>
        )}
      </div>
      <div className="min-w-0">
        <p
          className={cn(
            "truncate text-sm font-medium",
            playing && "text-accent-strong",
            unavailable && "text-text-faint line-through",
          )}
        >
          {track.title}
        </p>
        <p className="truncate text-xs text-text-muted">
          {unavailable ? "Unavailable" : (track.artist ?? "Unknown artist")}
        </p>
      </div>
    </button>
  );
}

/** A facet group (album/artist/folder/genre) rendered as a grid card. */
function GroupCard({
  group,
  facetIcon: Icon,
  showArt,
  onOpen,
}: {
  group: Group;
  facetIcon: LucideIcon;
  showArt: boolean;
  onOpen: () => void;
}) {
  return (
    <button type="button" onClick={onOpen} className="group flex h-full w-full flex-col gap-2 text-left">
      <div className="relative">
        {showArt ? (
          <Artwork
            art={group.art}
            seed={group.seed}
            label={group.label}
            rounded="rounded-xl"
            className="aspect-square w-full shadow-md ring-1 ring-white/5"
          />
        ) : (
          <span className="grid aspect-square w-full place-items-center rounded-xl bg-surface-overlay text-text-muted ring-1 ring-border">
            <Icon className="size-8" aria-hidden="true" />
          </span>
        )}
        <span className="absolute inset-0 grid place-items-center rounded-xl bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
          <span className="grid size-10 place-items-center rounded-full bg-accent text-on-accent shadow-lg">
            <Play className="size-5 fill-current" aria-hidden="true" />
          </span>
        </span>
      </div>
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{group.label}</p>
        <p className="truncate text-xs text-text-muted">{group.subtitle}</p>
      </div>
    </button>
  );
}

function SectionHeader({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="flex shrink-0 items-baseline gap-2">
      <h3 className="truncate text-sm font-semibold">{title}</h3>
      {subtitle && <span className="text-xs text-text-muted">{subtitle}</span>}
    </div>
  );
}

function Empty({ message }: { message: string }) {
  return <p className="px-2 py-10 text-center text-sm text-text-muted">{message}</p>;
}

/** A centred spinner shown while a source is loading for the first time. */
function LibraryLoading({ message }: { message: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <Loader2 className="size-7 animate-spin text-accent" aria-hidden="true" />
      <p className="text-sm text-text-muted">{message}</p>
    </div>
  );
}

/** Empty state, worded for whichever source is selected (and confirmed empty). */
function EmptySource({ source }: { source: SourceFilter }) {
  if (source === "phone") {
    return (
      <CenteredEmpty
        icon={Smartphone}
        title="No music on this phone"
        body="This phone is paired but no music came through. Open Hype Muzik on it and make sure your library has finished scanning."
      />
    );
  }
  if (source === "cloud") {
    return (
      <CenteredEmpty
        icon={Cloud}
        title="No music in your cloud"
        body="Your cloud account is connected, but no audio files were found in it."
      />
    );
  }
  if (source === "ytmusic") {
    return (
      <CenteredEmpty
        icon={SquarePlay}
        title="No playlists found"
        body="You're signed in, but this account has no playlists with tracks in them yet."
      />
    );
  }
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-surface-raised ring-1 ring-border">
        <Music2 className="size-7 text-text-faint" aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">No music yet</p>
        <p className="mt-1 max-w-xs text-sm text-text-muted">
          Add a folder in Settings, or connect your phone or a cloud account.
        </p>
      </div>
      <p className="text-xs text-text-faint">Settings → Music library → Add folder</p>
    </div>
  );
}

function CenteredEmpty({
  icon: Icon,
  title,
  body,
}: {
  icon: LucideIcon;
  title: string;
  body: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-surface-raised ring-1 ring-border">
        <Icon className="size-7 text-text-faint" aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">{title}</p>
        <p className="mt-1 max-w-xs text-sm text-text-muted">{body}</p>
      </div>
    </div>
  );
}

/** The sources that can be absent and offer a way to set them up. */
type ConnectKind = "phone" | "cloud" | "ytmusic";

const CONNECT_PROMPT: Record<
  ConnectKind,
  { icon: LucideIcon; title: string; body: string; action: string }
> = {
  phone: {
    icon: Smartphone,
    title: "No phone connected",
    body: "Pair a phone in Settings, then its music shows up here.",
    action: "Pair in Settings",
  },
  cloud: {
    icon: Cloud,
    title: "No cloud connected",
    body: "Connect Google Drive or Dropbox in Settings to stream your music here.",
    action: "Connect in Settings",
  },
  ytmusic: {
    icon: SquarePlay,
    title: "Not signed in to YouTube Music",
    body: "Sign in from Settings to browse and play your playlists here.",
    action: "Sign in from Settings",
  },
};

/** A connected source whose listing failed — says so, and offers the retry that
 *  actually addresses it. Deliberately not a `ConnectPrompt`: sending someone to
 *  Settings to connect an account that's already connected is a dead end. */
function SourceErrorPrompt({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-danger/10 ring-1 ring-danger/30">
        <CircleAlert className="size-7 text-danger" aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">Couldn't load your YouTube Music playlists</p>
        <p className="mt-1 max-w-sm text-sm text-text-muted">{message}</p>
      </div>
      <Button variant="primary" onClick={onRetry}>
        <RotateCw className="size-4" aria-hidden="true" />
        Retry
      </Button>
    </div>
  );
}

function ConnectPrompt({ kind, onConnect }: { kind: ConnectKind; onConnect: () => void }) {
  const { icon: Icon, title, body, action } = CONNECT_PROMPT[kind];
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-surface-raised ring-1 ring-border">
        <Icon className="size-7 text-text-faint" aria-hidden="true" />
      </div>
      <div>
        <p className="text-base font-medium">{title}</p>
        <p className="mt-1 max-w-xs text-sm text-text-muted">{body}</p>
      </div>
      <Button variant="primary" onClick={onConnect}>
        <ListMusic className="size-4" aria-hidden="true" />
        {action}
      </Button>
    </div>
  );
}
