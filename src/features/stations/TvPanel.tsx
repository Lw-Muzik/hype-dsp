import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { ChevronLeft, Search, Star, Tv } from "lucide-react";
import { Button } from "@/components/Button";
import { LayoutToggle, type LayoutMode } from "@/components/LayoutToggle";
import { VirtualList } from "@/components/VirtualList";
import { VirtualGrid } from "@/components/VirtualGrid";
import {
  tvByCategory,
  tvByCountry,
  tvCategories,
  tvCountries,
  tvFavoriteAdd,
  tvFavoriteRemove,
  tvFavoritesList,
  tvSearch,
  tvCheckAlive,
} from "@/lib/ipc";
import type { TvCategory, TvChannel, TvCountry } from "@/lib/types";
import { cn } from "@/lib/cn";
import { TvPlayer } from "./TvPlayer";
import { channelHealth, filterChannels, rankByHealth, type Health } from "./tvList";

type Mode = "browse" | "country" | "category" | "favorites";

/** How many channels of a list to health-probe. A list can hold hundreds; the
 *  top of it is what a viewer actually scans, so probe that many and let the
 *  filter narrow the rest. Bounds the cost to N requests per list open, cached
 *  an hour backend-side. */
const PROBE_CAP = 150;

/** ISO alpha-2 code → flag emoji (regional indicator symbols). */
function flag(code: string): string {
  return [...code.toUpperCase()]
    .map((c) => String.fromCodePoint(0x1f1e6 + c.charCodeAt(0) - 65))
    .join("");
}

/**
 * The TV kind of the Stations hub — world television from iptv-org, played
 * in-app in an embedded player. Browse by country or category, search the global
 * catalog, or keep favorites, in a list or grid.
 */
export function TvPanel({ active }: { active: boolean }) {
  const [mode, setMode] = useState<Mode>("browse");
  const [layout, setLayout] = useState<LayoutMode>("list");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<TvChannel[]>([]);
  const [favorites, setFavorites] = useState<TvChannel[]>([]);
  const [loading, setLoading] = useState(false);

  const [countries, setCountries] = useState<TvCountry[]>([]);
  const [countryQuery, setCountryQuery] = useState("");
  const [country, setCountry] = useState<TvCountry | null>(null);

  const [categories, setCategories] = useState<TvCategory[]>([]);
  const [category, setCategory] = useState<TvCategory | null>(null);

  const [list, setList] = useState<TvChannel[]>([]); // country/category results
  const [watching, setWatching] = useState<TvChannel | null>(null);

  // In-list filter (search WITHIN a loaded country/category/favorites list),
  // distinct from the global catalog search in browse mode.
  const [listFilter, setListFilter] = useState("");

  // Stream-health verdicts for the current list: which ids were probed, and
  // which came back alive. Dead = probed but not alive; the rest are unknown.
  const [probedIds, setProbedIds] = useState<Set<string>>(new Set());
  const [aliveIds, setAliveIds] = useState<Set<string>>(new Set());
  const [checking, setChecking] = useState(false);
  const healthRunRef = useRef(0);

  // Probe a freshly-loaded list and record the verdicts. Guarded by a run id so
  // a slow probe for a list the user already navigated away from can't overwrite
  // the current one's results.
  const runHealthCheck = useCallback((channels: TvChannel[]) => {
    setProbedIds(new Set());
    setAliveIds(new Set());
    const slice = channels.slice(0, PROBE_CAP);
    if (slice.length === 0) return;
    const run = ++healthRunRef.current;
    setChecking(true);
    tvCheckAlive(slice)
      .then((ids) => {
        if (healthRunRef.current !== run) return;
        setProbedIds(new Set(slice.map((c) => c.id)));
        setAliveIds(new Set(ids));
      })
      .catch(() => {})
      .finally(() => {
        if (healthRunRef.current === run) setChecking(false);
      });
  }, []);

  const loadedRef = useRef(false);
  // The scroll container the channel list virtualizes against.
  const scrollRef = useRef<HTMLDivElement>(null);
  const favIds = useMemo(() => new Set(favorites.map((f) => f.id)), [favorites]);

  const refreshFavorites = useCallback(() => {
    tvFavoritesList().then(setFavorites).catch(() => {});
  }, []);

  const doSearch = useCallback((q: string) => {
    setLoading(true);
    tvSearch(q)
      .then(setResults)
      .catch(() => setResults([]))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (!active || loadedRef.current) return;
    loadedRef.current = true;
    doSearch("");
    refreshFavorites();
    tvCategories().then(setCategories).catch(() => setCategories([]));
    tvCountries().then(setCountries).catch(() => setCountries([]));
    // Warm the hls.js chunk now (the TV tab is open) so the first channel click
    // doesn't pay the ~157 KB import on its critical path.
    void import("hls.js").catch(() => {});
  }, [active, doSearch, refreshFavorites]);

  const openCountry = (c: TvCountry) => {
    setCountry(c);
    setList([]);
    setListFilter("");
    setLoading(true);
    tvByCountry(c.code)
      .then((chs) => {
        setList(chs);
        runHealthCheck(chs);
      })
      .catch(() => setList([]))
      .finally(() => setLoading(false));
  };

  const openCategory = (c: TvCategory) => {
    setCategory(c);
    setList([]);
    setListFilter("");
    setLoading(true);
    tvByCategory(c.id)
      .then((chs) => {
        setList(chs);
        runHealthCheck(chs);
      })
      .catch(() => setList([]))
      .finally(() => setLoading(false));
  };

  const toggleFavorite = (c: TvChannel) => {
    const op = favIds.has(c.id) ? tvFavoriteRemove(c.id) : tvFavoriteAdd(c);
    op.then(refreshFavorites).catch(() => {});
  };

  const filteredCountries = useMemo(() => {
    const q = countryQuery.trim().toLowerCase();
    return q ? countries.filter((c) => c.name.toLowerCase().includes(q)) : countries;
  }, [countries, countryQuery]);

  const baseChannels =
    mode === "favorites" ? favorites : mode === "browse" ? results : list;

  // Health ordering + dimming applies to the browsed country/category lists —
  // the ones that mix live and dead. Browse (its own search) and favorites (the
  // user's own picks) are shown as-is.
  const healthed = mode === "country" || mode === "category";

  // In-list filter applies to every list except browse, which has the global
  // search box instead.
  const channels = useMemo(() => {
    const filtered =
      mode === "browse" ? baseChannels : filterChannels(baseChannels, listFilter);
    return healthed ? rankByHealth(filtered, probedIds, aliveIds) : filtered;
  }, [mode, baseChannels, listFilter, healthed, probedIds, aliveIds]);

  const healthOf = useCallback(
    (c: TvChannel): Health =>
      healthed ? channelHealth(c.id, probedIds, aliveIds) : "unknown",
    [healthed, probedIds, aliveIds],
  );

  const channelProps = {
    watchingId: watching?.id ?? null,
    favIds,
    layout,
    onPlay: setWatching,
    onToggleFavorite: toggleFavorite,
    healthOf,
    scrollRef,
  };

  return (
    <div className="flex h-full w-full flex-col gap-4">
      {watching && (
        <TvPlayer channel={watching} onClose={() => setWatching(null)} />
      )}

      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1 rounded-control border border-border bg-surface-raised p-1">
          {(["browse", "country", "category", "favorites"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => {
                setMode(m);
                if (m === "country") setCountry(null);
                if (m === "category") setCategory(null);
              }}
              className={cn(
                "rounded-[7px] px-3 py-1.5 text-sm capitalize transition-colors",
                mode === m
                  ? "bg-surface-overlay text-text"
                  : "text-text-muted hover:text-text",
              )}
            >
              {m}
            </button>
          ))}
        </div>

        {mode === "browse" && (
          <form
            className="flex flex-1 items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              doSearch(query);
            }}
          >
            <div className="flex flex-1 items-center gap-2 rounded-control border border-border bg-surface px-3 transition-colors focus-within:border-accent">
              <Search className="size-4 text-text-faint" aria-hidden="true" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search channels, categories, countries…"
                aria-label="Search TV channels"
                className="w-full bg-transparent py-2 text-sm placeholder:text-text-faint"
              />
            </div>
            <Button variant="secondary" type="submit" disabled={loading}>
              {loading ? "Searching…" : "Search"}
            </Button>
          </form>
        )}

        {mode === "country" && !country && (
          <div className="flex flex-1 items-center gap-2 rounded-control border border-border bg-surface px-3">
            <Search className="size-4 text-text-faint" aria-hidden="true" />
            <input
              value={countryQuery}
              onChange={(e) => setCountryQuery(e.target.value)}
              placeholder="Find a country…"
              aria-label="Search countries"
              className="w-full bg-transparent py-2 text-sm placeholder:text-text-faint"
            />
          </div>
        )}

        {mode === "country" && country && (
          <BackButton onClick={() => setCountry(null)}>
            {flag(country.code)} {country.name}
          </BackButton>
        )}

        {mode === "category" && category && (
          <BackButton onClick={() => setCategory(null)}>{category.name}</BackButton>
        )}

        {/* Filter WITHIN a loaded list (country/category/favorites). The global
            catalog search in browse mode is a separate, backend-backed box. */}
        {((mode === "country" && country) ||
          (mode === "category" && category) ||
          mode === "favorites") && (
          <div className="flex flex-1 items-center gap-2 rounded-control border border-border bg-surface px-3 transition-colors focus-within:border-accent">
            <Search className="size-4 text-text-faint" aria-hidden="true" />
            <input
              value={listFilter}
              onChange={(e) => setListFilter(e.target.value)}
              placeholder="Filter these channels…"
              aria-label="Filter channels in this list"
              className="w-full bg-transparent py-2 text-sm placeholder:text-text-faint"
            />
            {checking && (
              <span className="shrink-0 text-xs text-text-faint">Checking…</span>
            )}
          </div>
        )}

        <div className="ml-auto">
          <LayoutToggle value={layout} onChange={setLayout} />
        </div>
      </div>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised"
      >
        {mode === "country" && !country ? (
          <PickGrid items={filteredCountries.map((c) => ({ key: c.code, label: c.name, badge: flag(c.code) }))} onPick={(k) => openCountry(filteredCountries.find((c) => c.code === k)!)} empty="Loading countries…" />
        ) : mode === "category" && !category ? (
          <PickGrid items={categories.map((c) => ({ key: c.id, label: c.name }))} onPick={(k) => openCategory(categories.find((c) => c.id === k)!)} empty="Loading categories…" />
        ) : (
          <ChannelCollection
            channels={channels}
            loading={loading}
            emptyLabel={
              mode === "favorites"
                ? "No favorites yet. Star a channel to keep it here."
                : "No channels found."
            }
            {...channelProps}
          />
        )}
      </div>
    </div>
  );
}

function BackButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-control border border-border bg-surface-raised px-3 py-2 text-sm text-text-muted transition-colors hover:text-text"
    >
      <ChevronLeft className="size-4" aria-hidden="true" />
      <span>{children}</span>
    </button>
  );
}

/** A country/category picker grid (flag/label tiles). */
function PickGrid({
  items,
  onPick,
  empty,
}: {
  items: { key: string; label: string; badge?: string }[];
  onPick: (key: string) => void;
  empty: string;
}) {
  if (items.length === 0) {
    return (
      <div className="grid h-full min-h-[200px] place-items-center p-8 text-sm text-text-muted">
        {empty}
      </div>
    );
  }
  return (
    <ul className="grid grid-cols-2 gap-2 p-3 sm:grid-cols-3">
      {items.map((it) => (
        <li key={it.key}>
          <button
            type="button"
            onClick={() => onPick(it.key)}
            className="flex w-full items-center gap-3 rounded-control border border-border bg-surface px-3 py-2.5 text-left transition-colors hover:bg-surface-overlay"
          >
            <span className="text-2xl leading-none" aria-hidden="true">
              {it.badge ?? <Tv className="size-4 text-text-faint" />}
            </span>
            <span className="min-w-0 flex-1 truncate text-sm font-medium">{it.label}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

type ChannelProps = {
  channels: TvChannel[];
  loading: boolean;
  emptyLabel: string;
  layout: LayoutMode;
  watchingId: string | null;
  favIds: Set<string>;
  onPlay: (c: TvChannel) => void;
  onToggleFavorite: (c: TvChannel) => void;
  /** Probe verdict per channel, for dimming the dead ones. */
  healthOf: (c: TvChannel) => Health;
  /** The scroll container the list/grid virtualizes against. */
  scrollRef: RefObject<HTMLDivElement | null>;
};

// A country or category list runs to hundreds — a big one to ~1,500 — channels.
// Rendered whole, that many rows (each with an image) is what janks and flickers
// on scroll, so the list and grid are windowed, exactly like the music library.
const TV_ROW_H = 64;
const TV_GRID_MIN = 150;
const TV_GRID_TEXT = 52;

function ChannelCollection(props: ChannelProps) {
  const { channels, loading, emptyLabel, layout } = props;
  if (channels.length === 0) {
    return (
      <div className="flex h-full min-h-[200px] flex-col items-center justify-center gap-2 p-8 text-center">
        <Tv className="size-8 text-text-faint" aria-hidden="true" />
        <p className="text-sm text-text-muted">{loading ? "Loading…" : emptyLabel}</p>
      </div>
    );
  }
  return layout === "grid" ? <ChannelGrid {...props} /> : <ChannelList {...props} />;
}

function subtitleOf(c: TvChannel): string {
  return [c.group, c.country, c.quality].filter(Boolean).join(" · ") || "TV channel";
}

function ChannelList({ channels, watchingId, favIds, onPlay, onToggleFavorite, healthOf, scrollRef }: ChannelProps) {
  return (
    <VirtualList
      items={channels}
      rowHeight={TV_ROW_H}
      scrollRef={scrollRef}
      ariaLabel="TV channels"
      getKey={(c) => c.id}
      renderRow={(c) => (
        <div
          onClick={() => onPlay(c)}
          className={cn(
            "flex h-full cursor-pointer items-center gap-3 border-b border-border/60 px-4 transition-colors hover:bg-surface-overlay",
            watchingId === c.id && "bg-accent-muted/40",
            // Dead channels stay clickable (a probe can be wrong, and the player
            // will say so), just dimmed and sunk to the bottom.
            healthOf(c) === "dead" && "opacity-45",
          )}
        >
          <ChannelLogo src={c.logo} className="size-10" />
          <div className="min-w-0 flex-1">
            <p className={cn("truncate text-sm font-medium", watchingId === c.id && "text-accent-strong")}>
              {c.name}
            </p>
            <p className="truncate text-xs text-text-muted">
              {healthOf(c) === "dead" ? "Unavailable" : subtitleOf(c)}
            </p>
          </div>
          <FavButton isFav={favIds.has(c.id)} onClick={() => onToggleFavorite(c)} />
        </div>
      )}
    />
  );
}

function ChannelGrid({ channels, watchingId, favIds, onPlay, onToggleFavorite, healthOf, scrollRef }: ChannelProps) {
  return (
    <VirtualGrid
      items={channels}
      minColWidth={TV_GRID_MIN}
      textHeight={TV_GRID_TEXT}
      scrollRef={scrollRef}
      ariaLabel="TV channels"
      getKey={(c) => c.id}
      renderCell={(c) => (
        <div
          onClick={() => onPlay(c)}
          className={cn(
            "group relative flex h-full cursor-pointer flex-col gap-2 rounded-card border border-border bg-surface p-3 transition-colors hover:bg-surface-overlay",
            watchingId === c.id && "border-accent bg-accent-muted/30",
            healthOf(c) === "dead" && "opacity-45",
          )}
        >
          {/* Square tile, not 16:9 — a channel logo suits a square, and it's what
              lets the grid window against a fixed cell height. */}
          <div className="grid aspect-square w-full flex-1 place-items-center overflow-hidden rounded-control bg-surface-overlay">
            <ChannelLogo src={c.logo} className="size-14" contain />
          </div>
          <div className="min-w-0">
            <p className={cn("truncate text-sm font-medium", watchingId === c.id && "text-accent-strong")}>
              {c.name}
            </p>
            <p className="truncate text-xs text-text-muted">
              {healthOf(c) === "dead" ? "Unavailable" : subtitleOf(c)}
            </p>
          </div>
          <div className="absolute right-2 top-2">
            <FavButton
              isFav={favIds.has(c.id)}
              onClick={() => onToggleFavorite(c)}
              className="bg-black/30 backdrop-blur"
            />
          </div>
        </div>
      )}
    />
  );
}

function FavButton({
  isFav,
  onClick,
  className,
}: {
  isFav: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      aria-label={isFav ? "Remove favorite" : "Add favorite"}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-control transition-colors",
        isFav ? "text-warning" : "text-text-faint hover:text-text",
        className,
      )}
    >
      <Star className="size-4" fill={isFav ? "currentColor" : "none"} aria-hidden="true" />
    </button>
  );
}

/** A channel's logo with a graceful fallback to a TV glyph. */
function ChannelLogo({
  src,
  className,
  contain,
}: {
  src: string | null;
  className?: string;
  contain?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    return (
      <div className={cn("grid shrink-0 place-items-center rounded-md bg-surface-overlay text-text-faint", className)}>
        <Tv className="size-4" aria-hidden="true" />
      </div>
    );
  }
  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
      className={cn("shrink-0 rounded-md bg-surface-overlay object-contain", contain ? "max-h-full max-w-full" : "", className)}
    />
  );
}
