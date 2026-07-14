import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { ChevronLeft, Search, Star, Tv, X } from "lucide-react";
import { Button } from "@/components/Button";
import { toast } from "@/stores/toast";
import { ipcErrorMessage } from "@/lib/ipc";
import {
  tvByCategory,
  tvByCountry,
  tvCategories,
  tvCountries,
  tvFavoriteAdd,
  tvFavoriteRemove,
  tvFavoritesList,
  tvPlay,
  tvPlayerStatus,
  tvSearch,
  tvStop,
} from "@/lib/ipc";
import type { TvCategory, TvChannel, TvCountry } from "@/lib/types";
import { cn } from "@/lib/cn";

type Mode = "browse" | "country" | "category" | "favorites";

/** ISO alpha-2 code → flag emoji (regional indicator symbols). */
function flag(code: string): string {
  return [...code.toUpperCase()]
    .map((c) => String.fromCodePoint(0x1f1e6 + c.charCodeAt(0) - 65))
    .join("");
}

/**
 * The TV kind of the Stations hub — world television from iptv-org, played in a
 * native mpv window. Browse by country or category, search the global catalog,
 * or keep favorites. `active` gates the first data load and the player-status
 * poll so nothing runs while the Radio tab is showing.
 */
export function TvPanel({ active }: { active: boolean }) {
  const [mode, setMode] = useState<Mode>("browse");
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

  // First time the TV tab is shown, hydrate everything once.
  useEffect(() => {
    if (!active || loadedRef.current) return;
    loadedRef.current = true;
    doSearch("");
    refreshFavorites();
    tvCategories().then(setCategories).catch(() => setCategories([]));
    tvCountries().then(setCountries).catch(() => setCountries([]));
  }, [active, doSearch, refreshFavorites]);

  // Poll the native player while TV is active so the "now watching" bar clears
  // when the user closes the mpv window.
  useEffect(() => {
    if (!active || !watching) return;
    const timer = setInterval(() => {
      tvPlayerStatus()
        .then((running) => {
          if (!running) setWatching(null);
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(timer);
  }, [active, watching]);

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

  const play = (c: TvChannel) => {
    setWatching(c);
    tvPlay(c).catch((e) => {
      setWatching(null);
      toast.error(`Couldn't start TV: ${ipcErrorMessage(e)}`);
    });
  };

  const stop = () => {
    setWatching(null);
    tvStop().catch(() => {});
  };

  const toggleFavorite = (c: TvChannel) => {
    const op = favIds.has(c.id) ? tvFavoriteRemove(c.id) : tvFavoriteAdd(c);
    op.then(refreshFavorites).catch(() => {});
  };

  const filteredCountries = useMemo(() => {
    const q = countryQuery.trim().toLowerCase();
    return q ? countries.filter((c) => c.name.toLowerCase().includes(q)) : countries;
  }, [countries, countryQuery]);

  const channelProps = {
    watchingId: watching?.id ?? null,
    favIds,
    onPlay: play,
    onToggleFavorite: toggleFavorite,
  };

  return (
    <div className="flex h-full w-full flex-col gap-4">
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
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised">
        {mode === "country" && !country ? (
          <CountryGrid countries={filteredCountries} onPick={openCountry} />
        ) : mode === "category" && !category ? (
          <CategoryGrid categories={categories} onPick={openCategory} />
        ) : (
          <ChannelList
            channels={
              mode === "favorites"
                ? favorites
                : mode === "browse"
                  ? results
                  : list
            }
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

      {watching && (
        <div className="flex items-center gap-3 rounded-card border border-accent/40 bg-accent-muted/30 px-4 py-2.5">
          <span className="relative flex size-2 shrink-0">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-accent-strong/70" />
            <span className="relative inline-flex size-2 rounded-full bg-accent-strong" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium text-text">
              Now watching · {watching.name}
            </p>
            <p className="truncate text-xs text-text-muted">
              Playing in the native TV window.
            </p>
          </div>
          <button
            type="button"
            onClick={stop}
            aria-label="Stop TV"
            className="flex items-center gap-1.5 rounded-control border border-border bg-surface-raised px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
          >
            <X className="size-4" aria-hidden="true" />
            Stop
          </button>
        </div>
      )}
    </div>
  );
}

function BackButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: ReactNode;
}) {
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

function CountryGrid({
  countries,
  onPick,
}: {
  countries: TvCountry[];
  onPick: (c: TvCountry) => void;
}) {
  if (countries.length === 0) {
    return (
      <div className="grid h-full min-h-[200px] place-items-center p-8 text-sm text-text-muted">
        Loading countries…
      </div>
    );
  }
  return (
    <ul className="grid grid-cols-2 gap-2 p-3 sm:grid-cols-3">
      {countries.map((c) => (
        <li key={c.code}>
          <button
            type="button"
            onClick={() => onPick(c)}
            className="flex w-full items-center gap-3 rounded-control border border-border bg-surface px-3 py-2.5 text-left transition-colors hover:bg-surface-overlay"
          >
            <span className="text-2xl leading-none" aria-hidden="true">
              {flag(c.code)}
            </span>
            <span className="min-w-0 flex-1 truncate text-sm font-medium">{c.name}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function CategoryGrid({
  categories,
  onPick,
}: {
  categories: TvCategory[];
  onPick: (c: TvCategory) => void;
}) {
  if (categories.length === 0) {
    return (
      <div className="grid h-full min-h-[200px] place-items-center p-8 text-sm text-text-muted">
        Loading categories…
      </div>
    );
  }
  return (
    <ul className="grid grid-cols-2 gap-2 p-3 sm:grid-cols-3">
      {categories.map((c) => (
        <li key={c.id}>
          <button
            type="button"
            onClick={() => onPick(c)}
            className="flex w-full items-center gap-3 rounded-control border border-border bg-surface px-3 py-2.5 text-left transition-colors hover:bg-surface-overlay"
          >
            <Tv className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate text-sm font-medium">{c.name}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function ChannelList({
  channels,
  loading,
  emptyLabel,
  watchingId,
  favIds,
  onPlay,
  onToggleFavorite,
}: {
  channels: TvChannel[];
  loading: boolean;
  emptyLabel: string;
  watchingId: string | null;
  favIds: Set<string>;
  onPlay: (c: TvChannel) => void;
  onToggleFavorite: (c: TvChannel) => void;
}) {
  if (channels.length === 0) {
    return (
      <div className="flex h-full min-h-[200px] flex-col items-center justify-center gap-2 p-8 text-center">
        <Tv className="size-8 text-text-faint" aria-hidden="true" />
        <p className="text-sm text-text-muted">{loading ? "Loading…" : emptyLabel}</p>
      </div>
    );
  }
  return (
    <ul className="divide-y divide-border/60">
      {channels.map((c) => {
        const isPlaying = watchingId === c.id;
        const isFav = favIds.has(c.id);
        const subtitle =
          [c.group, c.country, c.quality].filter(Boolean).join(" · ") || "TV channel";
        return (
          <li key={c.id}>
            <div
              onClick={() => onPlay(c)}
              className={cn(
                "flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors hover:bg-surface-overlay",
                isPlaying && "bg-accent-muted/40",
              )}
            >
              <ChannelLogo src={c.logo} />
              <div className="min-w-0 flex-1">
                <p
                  className={cn(
                    "truncate text-sm font-medium",
                    isPlaying && "text-accent-strong",
                  )}
                >
                  {c.name}
                </p>
                <p className="truncate text-xs text-text-muted">{subtitle}</p>
              </div>
              <button
                type="button"
                aria-label={isFav ? "Remove favorite" : "Add favorite"}
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleFavorite(c);
                }}
                className={cn(
                  "flex size-8 shrink-0 items-center justify-center rounded-control transition-colors",
                  isFav ? "text-warning" : "text-text-faint hover:text-text",
                )}
              >
                <Star
                  className="size-4"
                  fill={isFav ? "currentColor" : "none"}
                  aria-hidden="true"
                />
              </button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

/** A channel's logo with a graceful fallback to a TV glyph. */
function ChannelLogo({ src }: { src: string | null }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    return (
      <div className="grid size-10 shrink-0 place-items-center rounded-md bg-surface-overlay text-text-faint">
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
      className="size-10 shrink-0 rounded-md bg-surface-overlay object-contain"
    />
  );
}
