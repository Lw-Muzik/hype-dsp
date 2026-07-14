import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, Radio, Search, Star } from "lucide-react";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  radioAfricanCountries,
  radioByCountry,
  radioFavoriteAdd,
  radioFavoriteRemove,
  radioFavoritesList,
  radioSearch,
} from "@/lib/ipc";
import type { RadioCountry, RadioStation } from "@/lib/types";
import { cn } from "@/lib/cn";

type Mode = "browse" | "africa" | "favorites";

/** ISO alpha-2 code → flag emoji (regional indicator symbols). */
function flag(code: string): string {
  return [...code.toUpperCase()]
    .map((c) => String.fromCodePoint(0x1f1e6 + c.charCodeAt(0) - 65))
    .join("");
}

// Fallback country list (the backend is authoritative) so the grid still shows
// if the command is unavailable. Kept in sync with commands/radio.rs.
const FALLBACK_COUNTRIES: RadioCountry[] = (
  [
    ["DZ", "Algeria"], ["AO", "Angola"], ["BJ", "Benin"], ["BW", "Botswana"],
    ["BF", "Burkina Faso"], ["BI", "Burundi"], ["CV", "Cabo Verde"],
    ["CM", "Cameroon"], ["CF", "Central African Republic"], ["TD", "Chad"],
    ["KM", "Comoros"], ["CG", "Congo"], ["CI", "Côte d'Ivoire"], ["CD", "DR Congo"],
    ["DJ", "Djibouti"], ["EG", "Egypt"], ["GQ", "Equatorial Guinea"],
    ["ER", "Eritrea"], ["SZ", "Eswatini"], ["ET", "Ethiopia"], ["GA", "Gabon"],
    ["GM", "Gambia"], ["GH", "Ghana"], ["GN", "Guinea"], ["GW", "Guinea-Bissau"],
    ["KE", "Kenya"], ["LS", "Lesotho"], ["LR", "Liberia"], ["LY", "Libya"],
    ["MG", "Madagascar"], ["MW", "Malawi"], ["ML", "Mali"], ["MR", "Mauritania"],
    ["MU", "Mauritius"], ["MA", "Morocco"], ["MZ", "Mozambique"], ["NA", "Namibia"],
    ["NE", "Niger"], ["NG", "Nigeria"], ["RW", "Rwanda"],
    ["ST", "São Tomé and Príncipe"], ["SN", "Senegal"], ["SC", "Seychelles"],
    ["SL", "Sierra Leone"], ["SO", "Somalia"], ["ZA", "South Africa"],
    ["SS", "South Sudan"], ["SD", "Sudan"], ["TZ", "Tanzania"], ["TG", "Togo"],
    ["TN", "Tunisia"], ["UG", "Uganda"], ["ZM", "Zambia"], ["ZW", "Zimbabwe"],
  ] as const
).map(([code, name]) => ({ code, name }));

/** The Radio kind of the Stations hub — internet radio through the DSP engine. */
export function RadioPanel() {
  const playRadio = useEngineStore((s) => s.playRadio);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [mode, setMode] = useState<Mode>("browse");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<RadioStation[]>([]);
  const [favorites, setFavorites] = useState<RadioStation[]>([]);
  const [loading, setLoading] = useState(false);

  // Africa browser
  const [countries, setCountries] = useState<RadioCountry[]>([]);
  const [countryQuery, setCountryQuery] = useState("");
  const [country, setCountry] = useState<RadioCountry | null>(null);
  const [countryStations, setCountryStations] = useState<RadioStation[]>([]);

  const favIds = useMemo(() => new Set(favorites.map((f) => f.id)), [favorites]);

  const refreshFavorites = useCallback(() => {
    radioFavoritesList().then(setFavorites).catch(() => {});
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
    radioAfricanCountries()
      .then((c) => setCountries(c.length > 0 ? c : FALLBACK_COUNTRIES))
      .catch(() => setCountries(FALLBACK_COUNTRIES));
  }, [doSearch, refreshFavorites]);

  const openCountry = (c: RadioCountry) => {
    setCountry(c);
    setCountryStations([]);
    setLoading(true);
    radioByCountry(c.code)
      .then(setCountryStations)
      .catch(() => setCountryStations([]))
      .finally(() => setLoading(false));
  };

  const toggleFavorite = (s: RadioStation) => {
    const op = favIds.has(s.id) ? radioFavoriteRemove(s.id) : radioFavoriteAdd(s);
    op.then(refreshFavorites).catch(() => {});
  };

  const filteredCountries = useMemo(() => {
    const q = countryQuery.trim().toLowerCase();
    return q ? countries.filter((c) => c.name.toLowerCase().includes(q)) : countries;
  }, [countries, countryQuery]);

  const stationProps = { nowPlaying, favIds, onPlay: playRadio, onToggleFavorite: toggleFavorite };

  return (
    <div className="flex h-full w-full flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1 rounded-control border border-border bg-surface-raised p-1">
          {(["browse", "africa", "favorites"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => {
                setMode(m);
                if (m === "africa") setCountry(null);
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
                placeholder="Search stations, genres, countries…"
                aria-label="Search radio stations"
                className="w-full bg-transparent py-2 text-sm placeholder:text-text-faint"
              />
            </div>
            <Button variant="secondary" type="submit" disabled={loading}>
              {loading ? "Searching…" : "Search"}
            </Button>
          </form>
        )}

        {mode === "africa" && !country && (
          <div className="flex flex-1 items-center gap-2 rounded-control border border-border bg-surface px-3">
            <Search className="size-4 text-text-faint" aria-hidden="true" />
            <input
              value={countryQuery}
              onChange={(e) => setCountryQuery(e.target.value)}
              placeholder="Find a country…"
              aria-label="Search African countries"
              className="w-full bg-transparent py-2 text-sm placeholder:text-text-faint"
            />
          </div>
        )}

        {mode === "africa" && country && (
          <button
            type="button"
            onClick={() => setCountry(null)}
            className="flex items-center gap-1.5 rounded-control border border-border bg-surface-raised px-3 py-2 text-sm text-text-muted transition-colors hover:text-text"
          >
            <ChevronLeft className="size-4" aria-hidden="true" />
            <span>{flag(country.code)} {country.name}</span>
          </button>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised">
        {mode === "africa" && !country ? (
          <CountryGrid countries={filteredCountries} onPick={openCountry} />
        ) : (
          <StationList
            stations={
              mode === "favorites"
                ? favorites
                : mode === "africa"
                  ? countryStations
                  : results
            }
            loading={loading}
            emptyLabel={
              mode === "favorites"
                ? "No favorites yet. Star a station to keep it here."
                : "No stations found."
            }
            {...stationProps}
          />
        )}
      </div>
    </div>
  );
}

function CountryGrid({
  countries,
  onPick,
}: {
  countries: RadioCountry[];
  onPick: (c: RadioCountry) => void;
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

function StationList({
  stations,
  loading,
  emptyLabel,
  nowPlaying,
  favIds,
  onPlay,
  onToggleFavorite,
}: {
  stations: RadioStation[];
  loading: boolean;
  emptyLabel: string;
  nowPlaying: string | null;
  favIds: Set<string>;
  onPlay: (s: RadioStation) => void;
  onToggleFavorite: (s: RadioStation) => void;
}) {
  if (stations.length === 0) {
    return (
      <div className="flex h-full min-h-[200px] flex-col items-center justify-center gap-2 p-8 text-center">
        <Radio className="size-8 text-text-faint" aria-hidden="true" />
        <p className="text-sm text-text-muted">{loading ? "Loading…" : emptyLabel}</p>
      </div>
    );
  }
  return (
    <ul className="divide-y divide-border/60">
      {stations.map((s) => {
        const isPlaying = nowPlaying === s.name;
        const isFav = favIds.has(s.id);
        return (
          <li key={s.id}>
            <div
              onClick={() => onPlay(s)}
              className={cn(
                "flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors hover:bg-surface-overlay",
                isPlaying && "bg-accent-muted/40",
              )}
            >
              <StationLogo src={s.favicon} />
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
                  {[s.genre, s.country].filter(Boolean).join(" · ") || "Radio station"}
                </p>
              </div>
              <button
                type="button"
                aria-label={isFav ? "Remove favorite" : "Add favorite"}
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleFavorite(s);
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

/** A station's logo (favicon) with a graceful fallback to a radio glyph. */
function StationLogo({ src }: { src: string | null }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    return (
      <div className="grid size-10 shrink-0 place-items-center rounded-md bg-surface-overlay text-text-faint">
        <Radio className="size-4" aria-hidden="true" />
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
