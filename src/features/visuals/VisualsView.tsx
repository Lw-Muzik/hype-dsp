import { useEffect, useMemo, useRef, useState } from "react";
import {
  AudioLines,
  ChevronLeft,
  ChevronRight,
  ListMusic,
  Maximize2,
  Search,
  Star,
  X,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { useVisualizerStore } from "@/stores/visualizer";
import { cn } from "@/lib/cn";
import { useButterchurn } from "./useButterchurn";

/** Strip butterchurn's author/prefix noise into a friendlier label. */
function prettyName(name: string): string {
  return name.replace(/^[_$\s]+/, "").replace(/\s*\(\d+\)\s*$/, "").trim() || name;
}

/**
 * Embedded MilkDrop visualizer (butterchurn) that fills the middle section and
 * dances to whatever's playing. A slide-in picker lets you browse presets and
 * star favorites; "Fullscreen" pops the higher-fidelity native window (or, when
 * that isn't bundled, takes the canvas fullscreen).
 */
export function VisualsView() {
  const route = routeById("visuals");
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const favorites = useVisualizerStore((s) => s.favorites);
  const toggleFavorite = useVisualizerStore((s) => s.toggleFavorite);
  const lastPreset = useVisualizerStore((s) => s.lastPreset);
  const setLastPreset = useVisualizerStore((s) => s.setLastPreset);
  const nativeAvailable = useVisualizerStore((s) => s.available);
  const probe = useVisualizerStore((s) => s.probe);
  const startNative = useVisualizerStore((s) => s.start);

  const [current, setCurrent] = useState<string | null>(lastPreset);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [query, setQuery] = useState("");

  const { ready, error, presetNames, loadPreset } = useButterchurn(
    canvasRef,
    lastPreset,
    (name) => {
      setCurrent(name);
      setLastPreset(name);
    },
  );

  // Probe native availability so we know whether Fullscreen pops the native
  // window or falls back to the canvas.
  useEffect(() => {
    probe();
  }, [probe]);

  const select = (name: string) => {
    loadPreset(name);
    setCurrent(name);
    setLastPreset(name);
  };

  const step = (dir: 1 | -1) => {
    if (presetNames.length === 0) return;
    const idx = current ? presetNames.indexOf(current) : -1;
    const next =
      presetNames[(idx + dir + presetNames.length) % presetNames.length];
    if (next) select(next);
  };

  const goFullscreen = () => {
    if (nativeAvailable) {
      void startNative();
    } else {
      void canvasRef.current?.parentElement?.requestFullscreen?.().catch(() => {});
    }
  };

  const favoriteSet = useMemo(() => new Set(favorites), [favorites]);
  const isFavorite = current ? favoriteSet.has(current) : false;

  // Picker order: favorites first, then the rest; filtered by the search box.
  const ordered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (n: string) =>
      q === "" || prettyName(n).toLowerCase().includes(q) || n.toLowerCase().includes(q);
    const favs = presetNames.filter((n) => favoriteSet.has(n) && match(n));
    const rest = presetNames.filter((n) => !favoriteSet.has(n) && match(n));
    return { favs, rest };
  }, [presetNames, favoriteSet, query]);

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        icon={route.icon}
        title={route.label}
        subtitle={route.tagline}
        actions={
          <div className="flex gap-2">
            <Button
              variant={pickerOpen ? "primary" : "secondary"}
              onClick={() => setPickerOpen((v) => !v)}
            >
              <ListMusic className="size-4" aria-hidden="true" />
              Presets
            </Button>
            <Button variant="secondary" onClick={goFullscreen}>
              <Maximize2 className="size-4" aria-hidden="true" />
              {nativeAvailable ? "Pop out" : "Fullscreen"}
            </Button>
          </div>
        }
      />

      {/* The visual stage: butterchurn fills this; overlays sit on top. */}
      <div className="relative min-h-0 flex-1 overflow-hidden rounded-card bg-black ring-1 ring-border">
        <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />

        {/* Loading / error states */}
        {!ready && !error && (
          <div className="absolute inset-0 grid place-items-center text-sm text-text-muted">
            <div className="flex items-center gap-2">
              <AudioLines className="size-4 animate-pulse text-accent" aria-hidden="true" />
              Loading visualizer…
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 grid place-items-center p-6 text-center">
            <div className="max-w-sm text-sm text-text-muted">
              <p className="mb-1 font-medium text-text">Visualizer unavailable</p>
              <p>{error}</p>
            </div>
          </div>
        )}

        {/* Bottom control bar: prev · current preset · favorite · next */}
        {ready && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-center justify-center gap-3 bg-gradient-to-t from-black/70 to-transparent p-4">
            <button
              type="button"
              onClick={() => step(-1)}
              aria-label="Previous preset"
              className="pointer-events-auto grid size-9 place-items-center rounded-full bg-white/10 text-text backdrop-blur transition-colors hover:bg-white/20"
            >
              <ChevronLeft className="size-5" aria-hidden="true" />
            </button>

            <button
              type="button"
              onClick={() => setPickerOpen(true)}
              title="Browse presets"
              className="pointer-events-auto max-w-[46%] truncate rounded-full bg-white/10 px-4 py-2 text-sm font-medium text-text backdrop-blur transition-colors hover:bg-white/20"
            >
              {current ? prettyName(current) : "—"}
            </button>

            <button
              type="button"
              onClick={() => current && toggleFavorite(current)}
              disabled={!current}
              aria-pressed={isFavorite}
              aria-label={isFavorite ? "Unfavorite preset" : "Favorite preset"}
              className={cn(
                "pointer-events-auto grid size-9 place-items-center rounded-full backdrop-blur transition-colors disabled:opacity-40",
                isFavorite
                  ? "bg-accent text-text hover:bg-accent-strong"
                  : "bg-white/10 text-text hover:bg-white/20",
              )}
            >
              <Star
                className={cn("size-4", isFavorite && "fill-current")}
                aria-hidden="true"
              />
            </button>

            <button
              type="button"
              onClick={() => step(1)}
              aria-label="Next preset"
              className="pointer-events-auto grid size-9 place-items-center rounded-full bg-white/10 text-text backdrop-blur transition-colors hover:bg-white/20"
            >
              <ChevronRight className="size-5" aria-hidden="true" />
            </button>
          </div>
        )}

        {/* Slide-in preset picker */}
        <div
          className={cn(
            "absolute inset-y-0 right-0 z-10 flex w-80 max-w-[80%] flex-col border-l border-border bg-[#0b0c10]/95 backdrop-blur transition-transform duration-200",
            pickerOpen ? "translate-x-0" : "translate-x-full",
          )}
          aria-hidden={!pickerOpen}
        >
          <div className="flex items-center justify-between gap-2 border-b border-border px-4 py-3">
            <span className="text-sm font-semibold">Presets</span>
            <button
              type="button"
              onClick={() => setPickerOpen(false)}
              aria-label="Close presets"
              className="grid size-7 place-items-center rounded-full text-text-muted transition-colors hover:bg-surface-raised hover:text-text"
            >
              <X className="size-4" aria-hidden="true" />
            </button>
          </div>

          <div className="px-4 py-3">
            <div className="flex items-center gap-2 rounded-control border border-border bg-surface px-3 py-2">
              <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search presets"
                className="w-full bg-transparent text-sm text-text placeholder:text-text-faint focus:outline-none"
              />
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
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
                    onSelect={() => select(name)}
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
                onSelect={() => select(name)}
                onToggleFavorite={() => toggleFavorite(name)}
              />
            ))}
            {ordered.favs.length === 0 && ordered.rest.length === 0 && (
              <p className="px-2 py-6 text-center text-sm text-text-muted">
                {presetNames.length === 0 ? "Loading presets…" : "No matches."}
              </p>
            )}
          </div>
        </div>
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
        active ? "bg-accent-muted" : "hover:bg-surface-raised",
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
