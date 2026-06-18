import { useCallback, useEffect, useMemo, useState } from "react";
import { Radio, Search, Star } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  radioFavoriteAdd,
  radioFavoriteRemove,
  radioFavoritesList,
  radioSearch,
} from "@/lib/ipc";
import type { RadioStation } from "@/lib/types";
import { cn } from "@/lib/cn";

type Mode = "browse" | "favorites";

export function RadioView() {
  const route = routeById("radio");
  const playRadio = useEngineStore((s) => s.playRadio);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [mode, setMode] = useState<Mode>("browse");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<RadioStation[]>([]);
  const [favorites, setFavorites] = useState<RadioStation[]>([]);
  const [loading, setLoading] = useState(false);

  const favIds = useMemo(() => new Set(favorites.map((f) => f.id)), [favorites]);

  const refreshFavorites = useCallback(() => {
    radioFavoritesList()
      .then(setFavorites)
      .catch(() => {});
  }, []);

  const doSearch = useCallback((q: string) => {
    setLoading(true);
    radioSearch(q)
      .then(setResults)
      .catch(() => setResults([]))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    doSearch("");
    refreshFavorites();
  }, [doSearch, refreshFavorites]);

  const toggleFavorite = (s: RadioStation) => {
    const op = favIds.has(s.id)
      ? radioFavoriteRemove(s.id)
      : radioFavoriteAdd(s);
    op.then(refreshFavorites).catch(() => {});
  };

  const stations = mode === "browse" ? results : favorites;

  return (
    <div className="mx-auto flex h-full w-full max-w-4xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="flex items-center gap-3">
        <div className="flex items-center gap-1 rounded-control border border-border bg-surface-raised p-1">
          {(["browse", "favorites"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
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
            <div className="flex flex-1 items-center gap-2 rounded-control border border-border bg-surface px-3">
              <Search className="size-4 text-text-faint" aria-hidden="true" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search stations, genres, countries…"
                aria-label="Search radio stations"
                className="w-full bg-transparent py-2 text-sm outline-none placeholder:text-text-faint"
              />
            </div>
            <Button variant="secondary" type="submit" disabled={loading}>
              {loading ? "Searching…" : "Search"}
            </Button>
          </form>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised">
        {stations.length === 0 ? (
          <div className="flex h-full min-h-[200px] flex-col items-center justify-center gap-2 p-8 text-center">
            <Radio className="size-8 text-text-faint" aria-hidden="true" />
            <p className="text-sm text-text-muted">
              {mode === "favorites"
                ? "No favorites yet. Star a station to keep it here."
                : loading
                  ? "Searching…"
                  : "No stations found."}
            </p>
          </div>
        ) : (
          <ul className="divide-y divide-border/60">
            {stations.map((s) => {
              const isPlaying = nowPlaying === s.name;
              const isFav = favIds.has(s.id);
              return (
                <li key={s.id}>
                  <div
                    onClick={() => playRadio(s)}
                    className={cn(
                      "flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors hover:bg-surface-overlay",
                      isPlaying && "bg-accent-muted/40",
                    )}
                  >
                    <div className="min-w-0 flex-1">
                      <p
                        className={cn(
                          "truncate text-sm font-medium",
                          isPlaying && "text-accent-strong",
                        )}
                      >
                        {s.name}
                      </p>
                      <p className="truncate text-xs text-text-muted">
                        {[s.genre, s.country].filter(Boolean).join(" · ") ||
                          "Radio station"}
                      </p>
                    </div>
                    <button
                      type="button"
                      aria-label={isFav ? "Remove favorite" : "Add favorite"}
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleFavorite(s);
                      }}
                      className={cn(
                        "flex size-8 items-center justify-center rounded-control transition-colors",
                        isFav
                          ? "text-warning"
                          : "text-text-faint hover:text-text",
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
        )}
      </div>
    </div>
  );
}
