import { create } from "zustand";
import {
  visualizerAvailable,
  visualizerSetPreset,
  visualizerStart,
  visualizerStop,
} from "@/lib/ipc";

/** Render settings for the native MilkDrop visualizer window. */
export interface VisualizerSettings {
  /** Frames per second the renderer targets. Higher is smoother but heavier. */
  fps: number;
  /** How reactive presets are to the beat (projectM beat sensitivity). */
  beat: number;
}

/** Slider bounds + defaults, shared with the Settings UI. */
export const VISUALIZER_LIMITS = {
  fps: { min: 15, max: 60, step: 5, default: 30 },
  beat: { min: 0.1, max: 5, step: 0.1, default: 1 },
} as const;

const LS_KEY = "hm.visualizer";

const DEFAULTS: VisualizerSettings = {
  fps: VISUALIZER_LIMITS.fps.default,
  beat: VISUALIZER_LIMITS.beat.default,
};

const clampTo = (
  n: unknown,
  { min, max }: { min: number; max: number },
  fallback: number,
): number =>
  typeof n === "number" && Number.isFinite(n)
    ? Math.min(max, Math.max(min, n))
    : fallback;

function loadSettings(): VisualizerSettings {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return DEFAULTS;
    const p = JSON.parse(raw) as Partial<VisualizerSettings>;
    return {
      fps: clampTo(p.fps, VISUALIZER_LIMITS.fps, DEFAULTS.fps),
      beat: clampTo(p.beat, VISUALIZER_LIMITS.beat, DEFAULTS.beat),
    };
  } catch {
    return DEFAULTS;
  }
}

function saveSettings(s: VisualizerSettings): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(s));
  } catch {
    // No storage (private mode) — settings just won't persist.
  }
}

/* ---- preset selection + favorites (persisted) ---------------------------- */

const PRESETS_KEY = "hm.visualizer.presets";

interface PresetPrefs {
  /** Starred preset names, shown first in the browser. */
  favorites: string[];
  /** The selected preset, restored across sessions. */
  current: string | null;
  /** Cut to a fresh preset each time the playing track changes. */
  autoChange: boolean;
}

const DEFAULT_PREFS: PresetPrefs = {
  favorites: [],
  current: null,
  autoChange: true,
};

function loadPresetPrefs(): PresetPrefs {
  try {
    const raw = localStorage.getItem(PRESETS_KEY);
    if (!raw) return DEFAULT_PREFS;
    const p = JSON.parse(raw) as Partial<PresetPrefs>;
    return {
      favorites: Array.isArray(p.favorites)
        ? p.favorites.filter((x): x is string => typeof x === "string")
        : [],
      current: typeof p.current === "string" ? p.current : null,
      autoChange: typeof p.autoChange === "boolean" ? p.autoChange : true,
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

function savePresetPrefs(p: PresetPrefs): void {
  try {
    localStorage.setItem(PRESETS_KEY, JSON.stringify(p));
  } catch {
    // No storage — favorites just won't persist.
  }
}

interface VisualizerStore {
  /** Whether the native sidecar is bundled in this build (probed once). */
  available: boolean;
  /** Whether the visualizer window is currently open. */
  running: boolean;
  /** Persisted render settings (applied at window launch). */
  settings: VisualizerSettings;

  /** Starred preset names (persisted). */
  favorites: string[];
  /** The selected preset name (persisted), shown in the window. */
  current: string | null;
  /** Cut to a fresh preset on every track change (persisted). */
  autoChangePreset: boolean;

  /** Probe sidecar availability — call once on mount. */
  probe: () => void;
  /** Open the visualizer window on the current preset + settings. */
  start: () => Promise<void>;
  /** Close the visualizer window. */
  stop: () => Promise<void>;
  /** Open if closed, close if open. */
  toggle: () => void;
  /** Select a preset: persist it and push it to the window if it's open. */
  selectPreset: (name: string) => void;
  /** Star / unstar a preset by name. */
  toggleFavorite: (name: string) => void;
  /** Enable/disable cutting to a new preset per track. */
  setAutoChangePreset: (on: boolean) => void;
  /** Persist a render-settings change (takes effect next window launch). */
  update: (patch: Partial<VisualizerSettings>) => void;
}

const initialPrefs = loadPresetPrefs();

export const useVisualizerStore = create<VisualizerStore>((set, get) => ({
  available: false,
  running: false,
  settings: loadSettings(),
  favorites: initialPrefs.favorites,
  current: initialPrefs.current,
  autoChangePreset: initialPrefs.autoChange,

  probe: () => {
    visualizerAvailable()
      .then((v) => set({ available: v }))
      .catch(() => set({ available: false }));
  },

  start: async () => {
    const { settings, current } = get();
    try {
      await visualizerStart({
        fps: settings.fps,
        beat: settings.beat,
        preset: current ?? undefined,
      });
      set({ running: true });
    } catch {
      set({ running: false });
    }
  },

  stop: async () => {
    try {
      await visualizerStop();
    } catch {
      // Treat as closed regardless of the result.
    }
    set({ running: false });
  },

  toggle: () => {
    const { running, start, stop } = get();
    void (running ? stop() : start());
  },

  selectPreset: (name) => {
    savePresetPrefs({
      favorites: get().favorites,
      current: name,
      autoChange: get().autoChangePreset,
    });
    set({ current: name });
    if (get().running) {
      void visualizerSetPreset(name).catch(() => {});
    }
  },

  toggleFavorite: (name) => {
    const has = get().favorites.includes(name);
    const favorites = has
      ? get().favorites.filter((n) => n !== name)
      : [...get().favorites, name];
    savePresetPrefs({
      favorites,
      current: get().current,
      autoChange: get().autoChangePreset,
    });
    set({ favorites });
  },

  setAutoChangePreset: (on) => {
    savePresetPrefs({
      favorites: get().favorites,
      current: get().current,
      autoChange: on,
    });
    set({ autoChangePreset: on });
  },

  update: (patch) => {
    const settings = { ...get().settings, ...patch };
    saveSettings(settings);
    set({ settings });
  },
}));
