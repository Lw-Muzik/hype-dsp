import { create } from "zustand";

export type ThemeChoice = "system" | "dynamic" | "light" | "dark";
/** What actually lands in `data-theme`. `system` is never written. */
export type ResolvedTheme = "dynamic" | "light" | "dark";

/** Slider bounds + defaults, shared with the Settings UI. */
export const THEME_LIMITS = {
  blur: { min: 8, max: 96, step: 1, default: 48 },
} as const;

const CHOICE_KEY = "hm.theme";
const BLUR_KEY = "hm.theme.blur";

const CHOICES: readonly ThemeChoice[] = ["system", "dynamic", "light", "dark"];

/**
 * The whole theme rule, as a pure function.
 *
 * Pure because tests run in node with no `matchMedia` — the OS preference
 * arrives as a plain boolean and the subscription lives in `watchPrefersDark`.
 */
export function resolveTheme(choice: ThemeChoice, prefersDark: boolean): ResolvedTheme {
  if (choice === "system") return prefersDark ? "dark" : "light";
  return choice;
}

export function parseChoice(raw: unknown): ThemeChoice {
  return CHOICES.includes(raw as ThemeChoice) ? (raw as ThemeChoice) : "system";
}

export function clampBlur(raw: unknown): number {
  const { min, max, default: fallback } = THEME_LIMITS.blur;
  if (typeof raw !== "number" || !Number.isFinite(raw)) return fallback;
  return Math.min(max, Math.max(min, raw));
}

function loadChoice(): ThemeChoice {
  try {
    return parseChoice(localStorage.getItem(CHOICE_KEY));
  } catch {
    return "system";
  }
}

function loadBlur(): number {
  try {
    const raw = localStorage.getItem(BLUR_KEY);
    return clampBlur(raw === null ? undefined : Number(raw));
  } catch {
    return THEME_LIMITS.blur.default;
  }
}

function save(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // No storage (private mode) — the choice just won't persist.
  }
}

/** Reads the OS preference once. Safe to call before the store exists. */
export function prefersDarkNow(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return true; // The app is dark-first; dark is the safer guess.
  }
}

interface ThemeStore {
  choice: ThemeChoice;
  blur: number;
  prefersDark: boolean;
  resolved: ResolvedTheme;
  setChoice: (choice: ThemeChoice) => void;
  setBlur: (blur: number) => void;
  setPrefersDark: (prefersDark: boolean) => void;
}

export const useThemeStore = create<ThemeStore>((set, get) => ({
  choice: loadChoice(),
  blur: loadBlur(),
  prefersDark: prefersDarkNow(),
  resolved: resolveTheme(loadChoice(), prefersDarkNow()),

  setChoice: (choice) => {
    save(CHOICE_KEY, choice);
    set({ choice, resolved: resolveTheme(choice, get().prefersDark) });
  },
  setBlur: (blur) => {
    const next = clampBlur(blur);
    save(BLUR_KEY, String(next));
    set({ blur: next });
  },
  // Called by the matchMedia listener, so `system` follows a live OS flip.
  setPrefersDark: (prefersDark) =>
    set({ prefersDark, resolved: resolveTheme(get().choice, prefersDark) }),
}));

/** Subscribes to OS appearance changes. Returns an unsubscribe. */
export function watchPrefersDark(onChange: (prefersDark: boolean) => void): () => void {
  try {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: MediaQueryListEvent) => onChange(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  } catch {
    return () => {};
  }
}
