import { useEffect, useMemo, useRef, useState } from "react";
import { Monitor, Search, Shuffle, Sparkles, Star, X } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { useVisualizerStore } from "@/stores/visualizer";
import { useEngineStore } from "@/stores/engine";
import { visualizerPresetNames } from "@/lib/ipc";
import { cn } from "@/lib/cn";

/** Strip author/prefix noise from a `.milk` name into a friendlier label. */
function prettyName(name: string): string {
  return name.replace(/^[_$\s]+/, "").trim() || name;
}

/** Pick a random preset to cut to — favorites first if any, never the current. */
function pickPreset(
  all: string[],
  favorites: string[],
  current: string | null,
): string | null {
  const favs = favorites.filter((f) => all.includes(f));
  const pool = favs.length > 0 ? favs : all;
  const choices = pool.length > 1 ? pool.filter((n) => n !== current) : pool;
  if (choices.length === 0) return null;
  return choices[Math.floor(Math.random() * choices.length)] ?? null;
}

/**
 * Visuals — a browser for the bundled MilkDrop (`.milk`) presets that drives the
 * native visualizer window. Selecting a preset shows it in the window live (when
 * open); "Auto" cuts to a fresh preset on every track change. The visuals
 * themselves render in the separate window (a webview can't host native GL).
 */
export function VisualsView() {
  const route = routeById("visuals");

  const available = useVisualizerStore((s) => s.available);
  const running = useVisualizerStore((s) => s.running);
  const current = useVisualizerStore((s) => s.current);
  const favorites = useVisualizerStore((s) => s.favorites);
  const autoChange = useVisualizerStore((s) => s.autoChangePreset);
  const probe = useVisualizerStore((s) => s.probe);
  const start = useVisualizerStore((s) => s.start);
  const stop = useVisualizerStore((s) => s.stop);
  const selectPreset = useVisualizerStore((s) => s.selectPreset);
  const toggleFavorite = useVisualizerStore((s) => s.toggleFavorite);
  const setAutoChange = useVisualizerStore((s) => s.setAutoChangePreset);

  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [names, setNames] = useState<string[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    probe();
  }, [probe]);

  useEffect(() => {
    if (!available) return;
    visualizerPresetNames()
      .then(setNames)
      .catch(() => setNames([]));
  }, [available]);

  // Cut to a fresh preset whenever the track changes (read inputs via refs so
  // this fires only on a track change). selectPreset pushes to the window if it
  // is open, and remembers the choice either way.
  const autoRef = useRef(autoChange);
  autoRef.current = autoChange;
  const namesRef = useRef(names);
  namesRef.current = names;
  const favRef = useRef(favorites);
  favRef.current = favorites;
  const curRef = useRef(current);
  curRef.current = current;
  const prevTrackRef = useRef<string | null>(nowPlaying);

  useEffect(() => {
    if (nowPlaying === prevTrackRef.current) return;
    prevTrackRef.current = nowPlaying;
    if (!autoRef.current || !nowPlaying || namesRef.current.length === 0) return;
    const next = pickPreset(namesRef.current, favRef.current, curRef.current);
    if (next) selectPreset(next);
  }, [nowPlaying, selectPreset]);

  const favoriteSet = useMemo(() => new Set(favorites), [favorites]);

  const ordered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (n: string) =>
      q === "" || prettyName(n).toLowerCase().includes(q) || n.toLowerCase().includes(q);
    const favs = names.filter((n) => favoriteSet.has(n) && match(n));
    const rest = names.filter((n) => !favoriteSet.has(n) && match(n));
    return { favs, rest };
  }, [names, favoriteSet, query]);

  if (!available) {
    return (
      <div className="flex h-full flex-col">
        <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />
        <div className="grid flex-1 place-items-center">
          <div className="max-w-sm text-center text-sm text-text-muted">
            <Sparkles className="mx-auto mb-3 size-6 text-text-faint" aria-hidden="true" />
            <p className="font-medium text-text">Visualizer not available</p>
            <p className="mt-1">
              This build doesn&rsquo;t include the native MilkDrop renderer. See
              docs for enabling the visualizer.
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        icon={route.icon}
        title={route.label}
        subtitle={route.tagline}
        actions={
          <div className="flex gap-2">
            <Button
              variant={autoChange ? "primary" : "secondary"}
              onClick={() => setAutoChange(!autoChange)}
              title="Cut to a new preset on every track change"
              aria-pressed={autoChange}
            >
              <Shuffle className="size-4" aria-hidden="true" />
              Auto
            </Button>
            {running ? (
              <Button variant="secondary" onClick={() => void stop()}>
                <X className="size-4" aria-hidden="true" />
                Close window
              </Button>
            ) : (
              <Button variant="primary" onClick={() => void start()}>
                <Monitor className="size-4" aria-hidden="true" />
                Open visualizer
              </Button>
            )}
          </div>
        }
      />

      {/* Status: which preset is showing + where */}
      <div className="mb-3 flex items-center gap-3 rounded-card border border-border bg-surface-raised px-4 py-3">
        <span
          className={cn(
            "size-2 shrink-0 rounded-full",
            running ? "bg-accent" : "bg-border-strong",
          )}
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 text-sm">
          <p className="truncate font-medium">
            {current ? prettyName(current) : "No preset selected"}
          </p>
          <p className="text-xs text-text-muted">
            {running
              ? "Showing in the visualizer window"
              : "Open the window to see it — selections apply live once it's open"}
          </p>
        </div>
        <span className="shrink-0 text-xs tabular-nums text-text-faint">
          {names.length.toLocaleString()} presets
        </span>
      </div>

      {/* Search */}
      <div className="mb-3 flex items-center gap-2 rounded-control border border-border bg-surface px-3 py-2">
        <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search presets"
          className="w-full bg-transparent text-sm text-text placeholder:text-text-faint focus:outline-none"
        />
      </div>

      {/* Preset list */}
      <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-border bg-surface-raised p-2">
        {ordered.favs.length > 0 && (
          <>
            <p className="px-2 py-1.5 text-xs font-medium uppercase tracking-wider text-text-faint">
              Favorites
            </p>
            {ordered.favs.map((name) => (
              <PresetRow
                key={name}
                name={name}
                active={name === current}
                favorite
                onSelect={() => selectPreset(name)}
                onToggleFavorite={() => toggleFavorite(name)}
              />
            ))}
            <div className="my-2 border-t border-border/60" />
          </>
        )}
        {ordered.rest.map((name) => (
          <PresetRow
            key={name}
            name={name}
            active={name === current}
            favorite={false}
            onSelect={() => selectPreset(name)}
            onToggleFavorite={() => toggleFavorite(name)}
          />
        ))}
        {ordered.favs.length === 0 && ordered.rest.length === 0 && (
          <p className="px-2 py-8 text-center text-sm text-text-muted">
            {names.length === 0 ? "Loading presets…" : "No matches."}
          </p>
        )}
      </div>
    </div>
  );
}

function PresetRow({
  name,
  active,
  favorite,
  onSelect,
  onToggleFavorite,
}: {
  name: string;
  active: boolean;
  favorite: boolean;
  onSelect: () => void;
  onToggleFavorite: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center gap-1 rounded-control pl-3 pr-1.5",
        active ? "bg-accent-muted" : "hover:bg-surface-overlay",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        className={cn(
          "min-w-0 flex-1 truncate py-2 text-left text-sm",
          active ? "font-medium text-accent-strong" : "text-text",
        )}
        title={name}
      >
        {prettyName(name)}
      </button>
      <button
        type="button"
        onClick={onToggleFavorite}
        aria-pressed={favorite}
        aria-label={favorite ? "Unfavorite" : "Favorite"}
        className={cn(
          "grid size-7 shrink-0 place-items-center rounded-full transition-colors",
          favorite
            ? "text-accent"
            : "text-text-faint opacity-0 group-hover:opacity-100 hover:text-text",
        )}
      >
        <Star className={cn("size-3.5", favorite && "fill-current")} aria-hidden="true" />
      </button>
    </div>
  );
}
