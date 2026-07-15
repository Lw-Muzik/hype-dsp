import { Suspense, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  AudioLines,
  Maximize2,
  Minimize2,
  Monitor,
  Search,
  Shuffle,
  Sparkles,
  Star,
  X,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { useVisualizerStore } from "@/stores/visualizer";
import { useEngineStore } from "@/stores/engine";
import { sceneList, sceneSelect, sceneSelected, visualizerPresetNames } from "@/lib/ipc";
import type { SceneInfo } from "@/lib/ipc";
import { BUILT_SCENES, SCENE_COMPONENTS } from "./scenes/registry";
import { Combobox } from "@/components/Combobox";
import type { ComboItem } from "@/components/Combobox";
import { cn } from "@/lib/cn";

type Tab = "scenes" | "milkdrop";
const TAB_KEY = "hm.visuals.tab";

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
 * Visuals — two ways to watch the music:
 *  - "In-app" scenes: Canvas/WebGL visualizers rendered right here (the engine's
 *    spectrum/beat drive them).
 *  - "MilkDrop": browse the bundled `.milk` presets and drive the native window.
 */
export function VisualsView() {
  const route = routeById("visuals");
  const [tab, setTab] = useState<Tab>(() => {
    const t = (() => {
      try {
        return localStorage.getItem(TAB_KEY);
      } catch {
        return null;
      }
    })();
    return t === "milkdrop" ? "milkdrop" : "scenes";
  });
  const switchTab = (t: Tab) => {
    setTab(t);
    try {
      localStorage.setItem(TAB_KEY, t);
    } catch {
      // No storage — tab just won't persist.
    }
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        icon={route.icon}
        title={route.label}
        subtitle={route.tagline}
        actions={
          <div className="flex rounded-control border border-border bg-surface p-0.5">
            <TabButton active={tab === "scenes"} onClick={() => switchTab("scenes")}>
              In-app
            </TabButton>
            <TabButton active={tab === "milkdrop"} onClick={() => switchTab("milkdrop")}>
              MilkDrop
            </TabButton>
          </div>
        }
      />
      {tab === "scenes" ? <ScenesPanel /> : <MilkDropPanel />}
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-[10px] px-3 py-1.5 text-sm font-medium transition-colors",
        active ? "bg-accent text-on-accent" : "text-text-muted hover:text-text",
      )}
    >
      {children}
    </button>
  );
}

/* ----------------------------------------------------------- in-app scenes */

function ScenesPanel() {
  const [scenes, setScenes] = useState<SceneInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    sceneList()
      .then((list) => setScenes(list.length > 0 ? list : BUILT_SCENES))
      .catch(() => setScenes(BUILT_SCENES));
    sceneSelected()
      .then((id) => setSelected(id || "radial-spectrum"))
      .catch(() => setSelected("radial-spectrum"));
  }, []);

  const select = (id: string) => {
    if (!(id in SCENE_COMPONENTS)) return; // not built yet
    setSelected(id);
    void sceneSelect(id).catch(() => {});
  };

  // Fullscreen the visualizer. We DON'T use the DOM Fullscreen API
  // (`element.requestFullscreen`) — macOS WKWebView disables element fullscreen
  // by default, so it silently rejects there (it works only on Windows'
  // WebView2). Instead we drive the OS window into fullscreen via Tauri and
  // expand the stage to cover the whole window with a CSS overlay, which behaves
  // identically on macOS and Windows. Escape (and the button) exits.
  const setStageFullscreen = (on: boolean) => {
    setFullscreen(on);
    void getCurrentWindow()
      .setFullscreen(on)
      .catch(() => {
        // Window fullscreen unavailable — the CSS overlay still fills the app
        // window, so the visualizer at least expands over the in-app chrome.
      });
    // Nudge scenes that only listen for window resize (R3F/ResizeObserver-based
    // ones already follow the container) to re-fit the new stage size.
    requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
  };

  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setStageFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // setStageFullscreen is stable enough for this handler's lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fullscreen]);

  const Scene = selected ? SCENE_COMPONENTS[selected] : undefined;

  // Searchable dropdown of every scene; built ones show their kind (2D/3D),
  // not-yet-built ones are tagged "soon" (selecting them is a no-op via `select`).
  const sceneItems: ComboItem[] = useMemo(
    () =>
      scenes.map((s) => ({
        id: s.id,
        label: s.name,
        sublabel: s.id in SCENE_COMPONENTS ? s.kind : "soon",
      })),
    [scenes],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div
        className={cn(
          "overflow-hidden bg-black",
          fullscreen
            ? "fixed inset-0 z-[100] rounded-none"
            : "relative min-h-0 flex-1 rounded-card ring-1 ring-border",
        )}
      >
        {Scene ? (
          <Suspense fallback={<StageMessage>Loading visualizer…</StageMessage>}>
            <Scene key={selected} />
          </Suspense>
        ) : (
          <StageMessage>
            {scenes.length === 0 ? "Loading…" : "This visualizer is coming soon."}
          </StageMessage>
        )}
        <button
          type="button"
          onClick={() => setStageFullscreen(!fullscreen)}
          title={fullscreen ? "Exit fullscreen (Esc)" : "Fullscreen"}
          aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          className="absolute right-3 top-3 grid size-9 place-items-center rounded-full bg-white/10 text-text backdrop-blur transition-colors hover:bg-white/20"
        >
          {fullscreen ? (
            <Minimize2 className="size-4" aria-hidden="true" />
          ) : (
            <Maximize2 className="size-4" aria-hidden="true" />
          )}
        </button>
      </div>

      {/* Scene picker — searchable dropdown */}
      <div className="w-full max-w-xs">
        <Combobox
          items={sceneItems}
          value={selected}
          onSelect={select}
          placeholder="Choose a visualizer…"
          searchPlaceholder="Search visualizers…"
          emptyText="No matching visualizers"
        />
      </div>
    </div>
  );
}

function StageMessage({ children }: { children: ReactNode }) {
  return (
    <div className="absolute inset-0 grid place-items-center text-sm text-text-muted">
      <div className="flex items-center gap-2">
        <AudioLines className="size-4 animate-pulse text-accent" aria-hidden="true" />
        {children}
      </div>
    </div>
  );
}

/* -------------------------------------------------- MilkDrop native window */

function MilkDropPanel() {
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
    const match = (n: string) => q === "" || n.toLowerCase().includes(q);
    const favs = names.filter((n) => favoriteSet.has(n) && match(n));
    const rest = names.filter((n) => !favoriteSet.has(n) && match(n));
    return { favs, rest };
  }, [names, favoriteSet, query]);

  if (!available) {
    return (
      <div className="grid flex-1 place-items-center">
        <div className="max-w-sm text-center text-sm text-text-muted">
          <Sparkles className="mx-auto mb-3 size-6 text-text-faint" aria-hidden="true" />
          <p className="font-medium text-text">MilkDrop window not available</p>
          <p className="mt-1">
            This build doesn&rsquo;t include the native MilkDrop renderer.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Window controls */}
      <div className="mb-3 flex items-center gap-3 rounded-card border border-border bg-surface-raised px-4 py-3">
        <span
          className={cn(
            "size-2 shrink-0 rounded-full",
            running ? "bg-accent" : "bg-border-strong",
          )}
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 text-sm">
          <p className="truncate font-medium">{current ?? "No preset selected"}</p>
          <p className="text-xs text-text-muted">
            {running
              ? "Showing in the MilkDrop window"
              : "Open the window — selections then apply live"}
          </p>
        </div>
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
            Close
          </Button>
        ) : (
          <Button variant="primary" onClick={() => void start()}>
            <Monitor className="size-4" aria-hidden="true" />
            Open window
          </Button>
        )}
      </div>

      <div className="mb-3 flex items-center gap-2 rounded-control border border-border bg-surface px-3 py-2 transition-colors focus-within:border-accent">
        <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search presets"
          className="w-full bg-transparent text-sm text-text placeholder:text-text-faint"
        />
        <span className="shrink-0 text-xs tabular-nums text-text-faint">
          {names.length.toLocaleString()}
        </span>
      </div>

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
        {name}
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
