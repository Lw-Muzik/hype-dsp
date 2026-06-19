import { create } from "zustand";
import {
  visualizerAvailable,
  visualizerStart,
  visualizerStop,
} from "@/lib/ipc";

/** User-tunable render settings for the MilkDrop visualizer sidecar. */
export interface VisualizerSettings {
  /** Frames per second the renderer targets. Higher is smoother but heavier. */
  fps: number;
  /** How reactive presets are to the beat (projectM beat sensitivity). */
  beat: number;
  /** Whether presets auto-advance over time. */
  autoCycle: boolean;
  /** Seconds each preset shows before auto-advancing (when auto-cycle is on). */
  cycleSecs: number;
}

/** Slider bounds + defaults, shared with the Settings UI. */
export const VISUALIZER_LIMITS = {
  fps: { min: 15, max: 60, step: 5, default: 30 },
  beat: { min: 0.1, max: 5, step: 0.1, default: 1 },
  cycleSecs: { min: 5, max: 120, step: 5, default: 20 },
} as const;

const LS_KEY = "hm.visualizer";

// projectM has no separate "lock current preset" arg — so to stop auto-cycling
// we hand it an effectively unreachable display duration. The user can still
// advance presets by hand with ←/→ in the visualizer window.
const NEVER_CYCLE_SECS = 1e9;

const DEFAULTS: VisualizerSettings = {
  fps: VISUALIZER_LIMITS.fps.default,
  beat: VISUALIZER_LIMITS.beat.default,
  autoCycle: true,
  cycleSecs: VISUALIZER_LIMITS.cycleSecs.default,
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
      autoCycle:
        typeof p.autoCycle === "boolean" ? p.autoCycle : DEFAULTS.autoCycle,
      cycleSecs: clampTo(p.cycleSecs, VISUALIZER_LIMITS.cycleSecs, DEFAULTS.cycleSecs),
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

/** Map settings to the sidecar launch args. */
const startArgs = (s: VisualizerSettings) => ({
  fps: s.fps,
  beat: s.beat,
  presetSecs: s.autoCycle ? s.cycleSecs : NEVER_CYCLE_SECS,
});

interface VisualizerStore {
  /** Whether the native sidecar is bundled in this build (probed once). */
  available: boolean;
  /** Whether the visualizer window is currently open. */
  running: boolean;
  /** Persisted render settings. */
  settings: VisualizerSettings;

  /** Probe sidecar availability — call once on mount. */
  probe: () => void;
  /** Open the visualizer window with the current settings. */
  start: () => Promise<void>;
  /** Close the visualizer window. */
  stop: () => Promise<void>;
  /** Open if closed, close if open. */
  toggle: () => void;
  /**
   * Persist a settings change. The sidecar reads its config once at launch, so
   * the new values take effect the next time the window opens; while it's open
   * use {@link VisualizerStore.start} ("Restart to apply") to relaunch it.
   */
  update: (patch: Partial<VisualizerSettings>) => void;
}

export const useVisualizerStore = create<VisualizerStore>((set, get) => ({
  available: false,
  running: false,
  settings: loadSettings(),

  probe: () => {
    visualizerAvailable()
      .then((v) => set({ available: v }))
      .catch(() => set({ available: false }));
  },

  start: async () => {
    try {
      // visualizer_start replaces any instance already open, so this doubles
      // as "restart with the latest settings".
      await visualizerStart(startArgs(get().settings));
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

  update: (patch) => {
    const settings = { ...get().settings, ...patch };
    saveSettings(settings);
    set({ settings });
  },
}));
