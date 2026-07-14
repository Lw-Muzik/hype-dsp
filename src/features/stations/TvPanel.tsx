import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { ChevronLeft, Search, Star, Tv } from "lucide-react";
import { Button } from "@/components/Button";
import { LayoutToggle, type LayoutMode } from "@/components/LayoutToggle";
import {
  tvByCategory,
  tvByCountry,
  tvCategories,
  tvCountries,
  tvFavoriteAdd,
  tvFavoriteRemove,
  tvFavoritesList,
  tvSearch,
} from "@/lib/ipc";
import type { TvCategory, TvChannel, TvCountry } from "@/lib/types";
import { cn } from "@/lib/cn";
import { TvPlayer } from "./TvPlayer";

type Mode = "browse" | "country" | "category" | "favorites";

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

  const loadedRef = useRef(false);
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
  }, [active, doSearch, refreshFavorites]);

  const openCountry = (c: TvCountry) => {
    setCountry(c);
    setList([]);
    setLoading(true);
    tvByCountry(c.code)
      .then(setList)
      .catch(() => setList([]))
      .finally(() => setLoading(false));
  };

  const openCategory = (c: TvCategory) => {
    setCategory(c);
    setList([]);
    setLoading(true);
    tvByCategory(c.id)
      .then(setList)
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

  const channels =
    mode === "favorites" ? favorites : mode === "browse" ? results : list;

  const channelProps = {
    watchingId: watching?.id ?? null,
    favIds,
    layout,
    onPlay: setWatching,
    onToggleFavorite: toggleFavorite,
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

        <div className="ml-auto">
          <LayoutToggle value={layout} onChange={setLayout} />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised">
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
};

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

function ChannelList({ channels, watchingId, favIds, onPlay, onToggleFavorite }: ChannelProps) {
  return (
    <ul className="divide-y divide-border/60">
      {channels.map((c) => {
        const isPlaying = watchingId === c.id;
        const isFav = favIds.has(c.id);
        return (
          <li key={c.id}>
            <div
              onClick={() => onPlay(c)}
              className={cn(
                "flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors hover:bg-surface-overlay",
                isPlaying && "bg-accent-muted/40",
              )}
            >
              <ChannelLogo src={c.logo} className="size-10" />
              <div className="min-w-0 flex-1">
                <p className={cn("truncate text-sm font-medium", isPlaying && "text-accent-strong")}>
                  {c.name}
                </p>
                <p className="truncate text-xs text-text-muted">{subtitleOf(c)}</p>
              </div>
              <FavButton isFav={isFav} onClick={() => onToggleFavorite(c)} />
            </div>
          </li>
        );
      })}
    </ul>
  );
}

function ChannelGrid({ channels, watchingId, favIds, onPlay, onToggleFavorite }: ChannelProps) {
  return (
    <ul className="grid grid-cols-2 gap-3 p-3 sm:grid-cols-3 lg:grid-cols-4">
      {channels.map((c) => {
        const isPlaying = watchingId === c.id;
        const isFav = favIds.has(c.id);
        return (
          <li key={c.id}>
            <div
              onClick={() => onPlay(c)}
              className={cn(
                "group relative flex cursor-pointer flex-col gap-2 rounded-card border border-border bg-surface p-3 transition-colors hover:bg-surface-overlay",
                isPlaying && "border-accent bg-accent-muted/30",
              )}
            >
              <div className="grid aspect-video w-full place-items-center overflow-hidden rounded-control bg-surface-overlay">
                <ChannelLogo src={c.logo} className="size-14" contain />
              </div>
              <div className="min-w-0">
                <p className={cn("truncate text-sm font-medium", isPlaying && "text-accent-strong")}>
                  {c.name}
                </p>
                <p className="truncate text-xs text-text-muted">{subtitleOf(c)}</p>
              </div>
              <div className="absolute right-2 top-2">
                <FavButton
                  isFav={isFav}
                  onClick={() => onToggleFavorite(c)}
                  className="bg-black/30 backdrop-blur"
                />
              </div>
            </div>
          </li>
        );
      })}
    </ul>
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
