# Dynamic Theme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a user-selectable theme — System / Dynamic / Light / Dark — where Dynamic paints the current track's cover art, blurred, behind the whole window, with a user-controlled blur radius.

**Architecture:** Themes are a token-value swap. `src/styles/index.css` defines semantic tokens via Tailwind v4 `@theme`, which emits utilities referencing `var(--color-*)`; redefining those variables under `:root[data-theme="…"]` re-points every utility with no component changes. The backdrop is a negative-z child of an isolated root, so chrome reveals the art purely through token alpha.

**Tech Stack:** React 19, TypeScript, Tailwind v4 (CSS-first `@theme`), Zustand, Vitest (node environment — no jsdom).

**Spec:** `docs/superpowers/specs/2026-07-15-dynamic-theme-design.md`. Read it before starting; it carries the rationale and the contrast maths behind every number here.

## Global Constraints

- **Branch:** `feat/dynamic-theme`, cut from `main`. `LayoutToggle`, `TvPlayer` and `src/features/stations/` exist only on `feat/stations-tv` — **they are not on this branch. Never reference them.**
- **Tokens only.** Components use `bg-surface`, `text-text-muted`, `border-border`. Never a raw hex in a `.tsx`.
- **`@theme` must stay plain.** `@theme inline` bakes values into utilities and silently breaks every override.
- **Tests run in node.** There is no vitest config and no jsdom. Export pure functions and test them directly, as `src/stores/engine.test.ts` does. Do not add jsdom.
- **localStorage keys:** `hm.` prefix. Every read wrapped in try/catch, every value clamped/validated on load. Never `tauri-plugin-store` (registered at `src-tauri/src/lib.rs:216` but dead — no JS dep, zero call sites).
- **Blur:** range 8–96px, default 48, step 1. Scrim `rgb(10 11 14 / 0.72)`. Crossfade 600ms **linear**.
- **Contrast targets:** body/muted text ≥ 4.5:1, faint (decorative) ≥ 3:1.
- **Commands:** `pnpm test` (vitest run), `pnpm exec tsc --noEmit`, `pnpm build`.

---

### Task 1: Palette tokens + the contrast test

The foundation. The test parses the real stylesheet, so it cannot drift from what ships.

**Files:**
- Modify: `src/styles/index.css:8-37`
- Test: `src/styles/palette.test.ts` (create)

**Interfaces:**
- Consumes: nothing.
- Produces: CSS custom properties `--color-canvas`, `--color-on-accent`, and `:root[data-theme="light"]` / `:root[data-theme="dynamic"]` blocks. Later tasks set `data-theme` on `<html>` to `dynamic | light | dark`.

- [ ] **Step 1: Write the failing test**

Create `src/styles/palette.test.ts`:

```ts
import { readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Asserts WCAG contrast over the REAL stylesheet. Re-declaring the palette here
 * would guard nothing — the copy would drift and the test would keep passing
 * while the app regressed. So we parse `index.css` itself.
 */
const CSS = readFileSync(
  fileURLToPath(new URL("./index.css", import.meta.url)),
  "utf8",
);

type Rgba = { r: number; g: number; b: number; a: number };

/** Parses `#rrggbb` or `rgb(r g b / a)` — the only two shapes this file uses. */
function parseColor(value: string): Rgba {
  const v = value.trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(v);
  if (hex) {
    const n = parseInt(hex[1]!, 16);
    return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255, a: 1 };
  }
  const rgb = /^rgb\(\s*(\d+)\s+(\d+)\s+(\d+)\s*(?:\/\s*([\d.]+)\s*)?\)$/i.exec(v);
  if (rgb) {
    return {
      r: +rgb[1]!, g: +rgb[2]!, b: +rgb[3]!,
      a: rgb[4] === undefined ? 1 : +rgb[4],
    };
  }
  throw new Error(`palette.test: cannot parse color ${JSON.stringify(value)}`);
}

/** All custom-property declarations inside the block opened by `selector`. */
function tokens(selector: string): Record<string, string> {
  const at = CSS.indexOf(selector);
  if (at === -1) throw new Error(`palette.test: no block for ${selector}`);
  const open = CSS.indexOf("{", at);
  const close = CSS.indexOf("}", open);
  const body = CSS.slice(open + 1, close);
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/(--[a-z-]+)\s*:\s*([^;]+);/g)) {
    out[m[1]!] = m[2]!.trim();
  }
  return out;
}

/** A theme's full token set: the `@theme` defaults with its overrides applied. */
function theme(name: "dark" | "light" | "dynamic"): Record<string, string> {
  const base = tokens("@theme");
  if (name === "dark") return base;
  return { ...base, ...tokens(`:root[data-theme="${name}"]`) };
}

function get(t: Record<string, string>, name: string): Rgba {
  const v = t[name];
  if (!v) throw new Error(`palette.test: missing ${name}`);
  return parseColor(v);
}

/** Composite `fg` (with its own alpha) over opaque `bg`. */
function over(fg: Rgba, bg: Rgba): Rgba {
  return {
    r: Math.round(fg.a * fg.r + (1 - fg.a) * bg.r),
    g: Math.round(fg.a * fg.g + (1 - fg.a) * bg.g),
    b: Math.round(fg.a * fg.b + (1 - fg.a) * bg.b),
    a: 1,
  };
}

function luminance({ r, g, b }: Rgba): number {
  const f = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function ratio(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const WHITE: Rgba = { r: 255, g: 255, b: 255, a: 1 };
const BLACK: Rgba = { r: 0, g: 0, b: 0, a: 1 };

describe("palette", () => {
  it.each(["dark", "light"] as const)("%s: text tiers meet WCAG", (name) => {
    const t = theme(name);
    const surface = get(t, "--color-surface");
    expect(ratio(get(t, "--color-text"), surface)).toBeGreaterThanOrEqual(4.5);
    expect(ratio(get(t, "--color-text-muted"), surface)).toBeGreaterThanOrEqual(4.5);
    // Faint is the decorative tier — large/incidental text only.
    expect(ratio(get(t, "--color-text-faint"), surface)).toBeGreaterThanOrEqual(3);
  });

  it.each(["dark", "light"] as const)("%s: accent is legible both ways", (name) => {
    const t = theme(name);
    // As text on a surface...
    expect(ratio(get(t, "--color-accent"), get(t, "--color-surface"))).toBeGreaterThanOrEqual(4.5);
    expect(ratio(get(t, "--color-accent-strong"), get(t, "--color-surface"))).toBeGreaterThanOrEqual(4.5);
    // ...and as a fill with on-accent text over it.
    expect(ratio(get(t, "--color-on-accent"), get(t, "--color-accent"))).toBeGreaterThanOrEqual(4.5);
  });

  it("canvas is always opaque — body must never let the page show through", () => {
    for (const name of ["dark", "light", "dynamic"] as const) {
      expect(get(theme(name), "--color-canvas").a).toBe(1);
    }
  });

  // The one that matters: Dynamic lifts the background by an unknown amount,
  // because the art is the user's. Both extremes must hold.
  it.each([
    ["white art", WHITE],
    ["black art", BLACK],
  ])("dynamic: text survives %s behind the scrim", (_label, art) => {
    const t = theme("dynamic");
    // Read from the stylesheet, same as every other token — the component uses
    // var(--hm-backdrop-scrim), so there is exactly one source of this value.
    const backdrop = over(get(t, "--hm-backdrop-scrim"), art);
    expect(ratio(get(t, "--color-text"), backdrop)).toBeGreaterThanOrEqual(4.5);
    expect(ratio(get(t, "--color-text-muted"), backdrop)).toBeGreaterThanOrEqual(4.5);
    expect(ratio(get(t, "--color-text-faint"), backdrop)).toBeGreaterThanOrEqual(3);
  });

  it("dynamic: chrome stays legible over the worst backdrop", () => {
    const t = theme("dynamic");
    const backdrop = over(get(t, "--hm-backdrop-scrim"), WHITE);
    const sidebar = over(get(t, "--color-surface-raised"), backdrop);
    expect(ratio(get(t, "--color-text"), sidebar)).toBeGreaterThanOrEqual(4.5);
    expect(ratio(get(t, "--color-text-muted"), sidebar)).toBeGreaterThanOrEqual(4.5);
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `pnpm test palette`
Expected: FAIL — `palette.test: no block for :root[data-theme="light"]`.

- [ ] **Step 3: Add the tokens**

In `src/styles/index.css`, add two tokens to the existing `@theme` block (keep every current value):

```css
@theme {
  /* Body's base. Separate from --color-surface because Dynamic gives that an
     alpha, and a translucent body would expose the page canvas behind the app. */
  --color-canvas: #0a0b0e;

  --color-surface: #0a0b0e;
  /* …existing tokens unchanged… */

  /* Text that sits ON an accent fill. Distinct from --color-text because accent
     is a mid-tone: on dark the fill is bright amber and wants dark text, on
     light the fill is dark amber and wants white. */
  --color-on-accent: #0a0b0e;
}
```

Then append the two theme blocks after the `:root { color-scheme: dark; }` rule:

```css
/*
 * Themes override the @theme defaults. Tailwind v4 emits utilities that
 * reference var(--color-*), so re-pointing the variable re-points every
 * utility — no component changes. `:root[data-theme=…]` (0,1,1) outranks
 * `:root` (0,1,0), so this lands without !important.
 *
 * Every value is asserted in palette.test.ts. Do not hand-tune them.
 */
:root[data-theme="light"] {
  color-scheme: light;

  --color-canvas: #f4f5f7;
  --color-surface: #f4f5f7; /* a soft grey base; pure white is harsh */
  --color-surface-raised: #ffffff; /* light UIs lift *toward* white */
  --color-surface-overlay: #ffffff;
  --color-border: #e3e5ea;
  --color-border-strong: #cdd1d9;

  --color-text: #16181d;
  --color-text-muted: #5a616e;
  --color-text-faint: #767d8a;

  /* The brand amber is unreadable as text on white, so accent flips: a dark
     amber fill with white on it. "Stronger" still means more emphasis — it is
     simply darker here rather than brighter. */
  --color-accent: #8a6000;
  --color-accent-strong: #6b4a00;
  --color-accent-muted: #fdf3d9;
  --color-on-accent: #ffffff;
}

/*
 * Dynamic is dark-based. It changes only what the backdrop forces:
 * the two chrome surfaces gain alpha so the art shows through, and the muted
 * tiers brighten because the backdrop lifts the effective background.
 * Overlays stay opaque — a translucent dropdown over moving art is unreadable.
 */
:root[data-theme="dynamic"] {
  color-scheme: dark;

  --color-surface: rgb(10 11 14 / 0.55);
  --color-surface-raised: rgb(20 22 28 / 0.55);

  --color-text-muted: #c5cbd6;
  --color-text-faint: #a7aeba;

  /* The backdrop's only darkening step. ThemeBackdrop reads this via var(), and
     palette.test.ts reads it from this file — one source, so the contrast the
     component depends on is the contrast that gets asserted. */
  --hm-backdrop-scrim: rgb(10 11 14 / 0.72);
}
```

Also give the blur variable a boot value on the existing `:root` rule:

```css
:root {
  color-scheme: dark;
  /* Providers sets this from the store on mount. Until then the var would be
     undefined, which makes blur(var(…)) invalid and flashes unblurred art.
     Keep in sync with THEME_LIMITS.blur.default. */
  --hm-backdrop-blur: 48px;
}
```

Finally, point `body` at the opaque canvas — change `background: var(--color-surface);` to:

```css
body {
  background: var(--color-canvas);
  /* …rest unchanged… */
}
```

- [ ] **Step 4: Run the test**

Run: `pnpm test palette`
Expected: PASS — all suites green.

- [ ] **Step 5: Verify the build still compiles the CSS**

Run: `pnpm build`
Expected: exit 0. Tailwind must not error on the new blocks.

- [ ] **Step 6: Commit**

```bash
git add src/styles/index.css src/styles/palette.test.ts
git commit -m "feat(theme): light + dynamic palettes, contrast-tested

Adds --color-canvas (opaque body base) and --color-on-accent, plus
[data-theme] blocks for light and dynamic. The test parses index.css
itself rather than re-declaring the palette, so it cannot drift.

Dynamic brightens its own muted tiers: the backdrop lifts the effective
background, and the dark tiers fail against bright cover art at any
scrim."
```

---

### Task 2: The accent split (fixes a live contrast bug)

`bg-accent` pairs with `text-surface` in 8 places. That works only because `surface` is near-black; Task 1 made it near-white in Light, so those are now broken. `Button` primary is worse — it is `bg-accent text-text`, **1.58:1**, broken in the theme shipping today.

**Files:**
- Modify: `src/components/Button.tsx:12`
- Modify: `src/features/player/CategoryChips.tsx:35`, `src/components/NowPlayingBar.tsx:205`, `src/features/player/AlbumDeck.tsx:219`, `src/features/player/MusicLibrary.tsx:459,494,558,593,646`
- Test: `src/styles/palette.test.ts` (extend)

**Interfaces:**
- Consumes: `--color-on-accent` from Task 1.
- Produces: nothing new. After this, no component pairs `bg-accent` with `text-surface` or `text-text`.

- [ ] **Step 1: Write the failing test**

In `src/styles/palette.test.ts`, extend the existing `node:fs` import to
`import { readFileSync, readdirSync, statSync } from "node:fs";` and add
`import { join } from "node:path";` beside it. Then append the new suite at the
end of the file:

```ts
function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return tsxFiles(full);
    return full.endsWith(".tsx") ? [full] : [];
  });
}

describe("accent usage", () => {
  // `text-surface` on an accent fill only ever worked because surface was
  // near-black. Under [data-theme=light] it is near-white, so amber-on-white.
  // The correct pairing is text-on-accent, which flips with the theme.
  it("no element pairs an accent fill with surface/text colours", () => {
    const src = fileURLToPath(new URL("../", import.meta.url));
    const offenders: string[] = [];
    for (const file of tsxFiles(src)) {
      const body = readFileSync(file, "utf8");
      // Any string literal holding both, wherever it sits. Anchoring on
      // `className=` would miss the common shapes — these classes live inside
      // cn(...) and ternaries, not directly after the attribute.
      for (const m of body.matchAll(/["'`]([^"'`\n]*bg-accent[^"'`\n]*)["'`]/g)) {
        const cls = m[1]!;
        // (?![\w-]) matters: \btext-text\b also matches `text-text-muted`,
        // which is a perfectly good pairing, and would fail this test forever.
        if (/\btext-(?:surface|text)(?![\w-])/.test(cls)) {
          offenders.push(`${file.replace(src, "")}: ${cls.trim().slice(0, 60)}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `pnpm test palette`
Expected: FAIL — `offenders` lists `components/Button.tsx` and the `text-surface` sites.

Note: the regex only catches pairings inside a single class string. It is a safety net for the obvious shape, not a proof. Step 3 still visits each of the 9 sites by hand.

- [ ] **Step 3: Fix Button**

`src/components/Button.tsx:12` — replace the `primary` variant:

```ts
const variants: Record<Variant, string> = {
  // text-on-accent, not text-text: the fill is a mid-tone amber, and light text
  // on it measures 1.58:1. This token flips with the theme so the pairing holds.
  primary: "bg-accent text-on-accent hover:bg-accent-strong",
  secondary:
    "border border-border bg-surface-raised text-text hover:bg-surface-overlay",
  ghost: "text-text-muted hover:bg-surface-raised hover:text-text",
};
```

- [ ] **Step 4: Fix the 8 `text-surface` sites**

In each file below, replace `text-surface` with `text-on-accent` **only** in the class string that also contains `bg-accent`. Leave every other `text-surface` alone.

- `src/features/player/CategoryChips.tsx:35` — `"bg-accent text-surface"` → `"bg-accent text-on-accent"`
- `src/components/NowPlayingBar.tsx:205`
- `src/features/player/AlbumDeck.tsx:219`
- `src/features/player/MusicLibrary.tsx:459`
- `src/features/player/MusicLibrary.tsx:494`
- `src/features/player/MusicLibrary.tsx:558`
- `src/features/player/MusicLibrary.tsx:593`
- `src/features/player/MusicLibrary.tsx:646`

Confirm none remain:

```bash
grep -rn "bg-accent" src --include="*.tsx" | grep -E "text-(surface|text)\b"
```
Expected: no output.

- [ ] **Step 5: Run the tests**

Run: `pnpm test palette && pnpm exec tsc --noEmit`
Expected: PASS, exit 0.

- [ ] **Step 6: Commit**

```bash
git add src/components/Button.tsx src/features/player/CategoryChips.tsx \
  src/components/NowPlayingBar.tsx src/features/player/AlbumDeck.tsx \
  src/features/player/MusicLibrary.tsx src/styles/palette.test.ts
git commit -m "fix(ui): pair accent fills with text-on-accent

Button primary was bg-accent text-text -- near-white on amber, 1.58:1,
broken in the dark theme shipping today. The 8 text-surface pairings
worked only because surface is near-black; light flips it to near-white
and they become amber-on-white.

text-on-accent flips with the theme, so both hold. A test now fails if
an accent fill is paired with a surface/text colour again."
```

---

### Task 3: Theme store

**Files:**
- Create: `src/stores/theme.ts`
- Test: `src/stores/theme.test.ts` (create)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `type ThemeChoice = "system" | "dynamic" | "light" | "dark"`
  - `type ResolvedTheme = "dynamic" | "light" | "dark"`
  - `resolveTheme(choice: ThemeChoice, prefersDark: boolean): ResolvedTheme`
  - `THEME_LIMITS = { blur: { min: 8, max: 96, step: 1, default: 48 } } as const`
  - `parseChoice(raw: unknown): ThemeChoice` and `clampBlur(raw: unknown): number` — exported so the validation rules are testable in node
  - `prefersDarkNow(): boolean` — reads the OS preference once
  - `watchPrefersDark(onChange: (prefersDark: boolean) => void): () => void` — returns an unsubscribe
  - `useThemeStore` — Zustand store: `{ choice, blur, prefersDark, resolved, setChoice(c), setBlur(n), setPrefersDark(b) }`
  - `loadChoice` / `loadBlur` are **private** to the store — the exported `parseChoice` / `clampBlur` carry the logic worth testing; the loaders are just their try/catch wrappers.

- [ ] **Step 1: Write the failing test**

Create `src/stores/theme.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { resolveTheme, THEME_LIMITS, clampBlur, parseChoice } from "@/stores/theme";

describe("resolveTheme", () => {
  it("maps system onto the OS preference", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("passes explicit choices through, ignoring the OS", () => {
    for (const prefersDark of [true, false]) {
      expect(resolveTheme("dark", prefersDark)).toBe("dark");
      expect(resolveTheme("light", prefersDark)).toBe("light");
      // Dynamic is dark-based but is its own theme -- never resolves to dark.
      expect(resolveTheme("dynamic", prefersDark)).toBe("dynamic");
    }
  });
});

describe("parseChoice", () => {
  it("accepts the four known values", () => {
    for (const c of ["system", "dynamic", "light", "dark"] as const) {
      expect(parseChoice(c)).toBe(c);
    }
  });

  it("falls back to system for anything else", () => {
    // Values from a future/older build, or a user editing localStorage.
    for (const bad of [null, "", "DARK", "solarized", "0", "{}"]) {
      expect(parseChoice(bad)).toBe("system");
    }
  });
});

describe("clampBlur", () => {
  it("keeps in-range values", () => {
    expect(clampBlur(48)).toBe(48);
    expect(clampBlur(THEME_LIMITS.blur.min)).toBe(THEME_LIMITS.blur.min);
    expect(clampBlur(THEME_LIMITS.blur.max)).toBe(THEME_LIMITS.blur.max);
  });

  it("clamps out-of-range values to the bounds", () => {
    expect(clampBlur(-10)).toBe(THEME_LIMITS.blur.min);
    expect(clampBlur(1000)).toBe(THEME_LIMITS.blur.max);
  });

  it("falls back to the default for non-numbers", () => {
    // The floor is deliberately not 0: a sharp cover behind the library
    // collides with UI text. NaN/undefined must not become 0 by accident.
    for (const bad of [NaN, Infinity, undefined, null, "48", {}]) {
      expect(clampBlur(bad)).toBe(THEME_LIMITS.blur.default);
    }
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `pnpm test theme`
Expected: FAIL — cannot resolve `@/stores/theme`.

- [ ] **Step 3: Write the store**

Create `src/stores/theme.ts`:

```ts
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
```

- [ ] **Step 4: Run the tests**

Run: `pnpm test theme && pnpm exec tsc --noEmit`
Expected: PASS, exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/stores/theme.ts src/stores/theme.test.ts
git commit -m "feat(theme): theme store with a pure resolver

resolveTheme(choice, prefersDark) is pure so the system-follows-OS rule
is testable in node -- this repo has no vitest config and no jsdom, and
the house convention is to export pure functions. The matchMedia
subscription lives beside it in watchPrefersDark.

Persistence is hand-rolled localStorage with clamp-on-load, matching
stores/ui.ts and stores/visualizer.ts."
```

---

### Task 4: `reducedMotion` + `ThemeBackdrop`

The backdrop. `reducedMotion` is promoted here because this is its third consumer.

**Files:**
- Create: `src/lib/reducedMotion.ts`
- Modify: `src/features/player/AlbumDeck.tsx:21-23`, `src/features/player/LyricsView.tsx:10-12`
- Create: `src/features/theme/ThemeBackdrop.tsx`
- Test: `src/features/theme/backdropSource.test.ts` (create)

**Interfaces:**
- Consumes: `useThemeStore`, `THEME_LIMITS` (Task 3); `coverGradient` from `src/lib/cover.ts`.
- Produces:
  - `prefersReducedMotion(): boolean` from `@/lib/reducedMotion`
  - `backdropSource(meta): { kind: "art"; url: string } | { kind: "gradient"; css: string } | null` from `@/features/theme/backdropSource`
  - `<ThemeBackdrop />` — default-exported React component, no props.

- [ ] **Step 1: Write the failing test**

The layering is CSS and gets verified by eye; the *decision* of what to paint is pure logic, so that is what we test.

Create `src/features/theme/backdropSource.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { backdropSource } from "@/features/theme/backdropSource";
import type { TrackMeta } from "@/lib/types";

const meta = (over: Partial<TrackMeta> = {}): TrackMeta => ({
  title: "Song", artist: "Artist", album: "Album", cover: null, ...over,
});

describe("backdropSource", () => {
  it("paints the cover when there is one", () => {
    expect(backdropSource(meta({ cover: "data:image/jpeg;base64,AAA" })))
      .toEqual({ kind: "art", url: "data:image/jpeg;base64,AAA" });
  });

  it("falls back to the same gradient Artwork shows", () => {
    // Not a placeholder colour: matching Artwork means the backdrop and the
    // on-screen cover agree for tracks with no embedded art.
    const got = backdropSource(meta({ album: "Kind of Blue" }));
    expect(got?.kind).toBe("gradient");
    expect(got).toEqual(backdropSource(meta({ album: "Kind of Blue" })));
  });

  it("seeds the gradient from album, falling back to title", () => {
    const byAlbum = backdropSource(meta({ album: "A", title: "T" }));
    const byTitle = backdropSource(meta({ album: null, title: "A" }));
    expect(byAlbum).toEqual(byTitle);
  });

  it("paints nothing when nothing is playing", () => {
    expect(backdropSource(null)).toBeNull();
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `pnpm test backdropSource`
Expected: FAIL — cannot resolve `@/features/theme/backdropSource`.

- [ ] **Step 3: Write the source rule**

Create `src/features/theme/backdropSource.ts`:

```ts
import { coverGradient } from "@/lib/cover";
import type { TrackMeta } from "@/lib/types";

export type BackdropSource =
  | { kind: "art"; url: string }
  | { kind: "gradient"; css: string };

/**
 * What the backdrop should paint for `meta`.
 *
 * `null` means paint nothing — the theme's plain surface shows. Note this is
 * only reached when nothing is playing: a *playing* track with no embedded art
 * gets the same deterministic gradient `Artwork` renders, so the backdrop and
 * the cover on screen always agree.
 */
export function backdropSource(meta: TrackMeta | null): BackdropSource | null {
  if (!meta) return null;
  if (meta.cover) return { kind: "art", url: meta.cover };
  return { kind: "gradient", css: coverGradient(meta.album || meta.title || "") };
}
```

- [ ] **Step 4: Run the test**

Run: `pnpm test backdropSource`
Expected: PASS.

- [ ] **Step 5: Promote `reducedMotion`**

Create `src/lib/reducedMotion.ts`:

```ts
/**
 * Whether the user asked for less motion.
 *
 * Read once at import, matching the existing call sites. The CSS blanket rule
 * in styles/index.css already collapses animation/transition timing; this is
 * for JS-driven motion that CSS can't reach.
 */
export const prefersReducedMotion: boolean = (() => {
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
})();
```

In `src/features/player/AlbumDeck.tsx`, delete the local const at lines 21-23 and import instead:

```ts
import { prefersReducedMotion } from "@/lib/reducedMotion";
```

Rename the local usages from `reduceMotion` to `prefersReducedMotion` (or alias on import: `import { prefersReducedMotion as reduceMotion } from "@/lib/reducedMotion";` — prefer the alias to keep the diff small).

Do the same in `src/features/player/LyricsView.tsx` (lines 10-12).

Verify no copies remain:

```bash
grep -rn "prefers-reduced-motion" src --include="*.tsx" --include="*.ts"
```
Expected: only `src/lib/reducedMotion.ts`.

- [ ] **Step 6: Write `ThemeBackdrop`**

Create `src/features/theme/ThemeBackdrop.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { useEngineStore } from "@/stores/engine";
import { useThemeStore } from "@/stores/theme";
import { prefersReducedMotion } from "@/lib/reducedMotion";
import { backdropSource, type BackdropSource } from "./backdropSource";

const FADE_MS = 600;

/** One art layer. `show` drives the crossfade. */
function Layer({ source, show }: { source: BackdropSource | null; show: boolean }) {
  if (!source) return null;
  const art =
    source.kind === "art"
      ? { backgroundImage: `url("${source.url}")`, backgroundSize: "cover", backgroundPosition: "center" }
      : { background: source.css };
  return (
    // The wrapper is promoted, NOT the blurred child. Promoting the blurred
    // element would force the GPU to re-blur its texture every frame of the
    // fade; promoting the parent lets the blur rasterise once and be reused.
    <div
      className="absolute inset-0"
      style={{
        willChange: "transform",
        opacity: show ? 1 : 0,
        // Linear: an eased crossfade dips visibly in the middle.
        transition: prefersReducedMotion ? undefined : `opacity ${FADE_MS}ms linear`,
      }}
      aria-hidden="true"
    >
      <div
        className="absolute"
        style={{
          // blur()'s length is a standard deviation, so it bleeds ~3x that far
          // and samples transparent pixels past the edge, fading them. Oversize
          // by 3σ to put the fade off-screen. (scale() would magnify it, since
          // transform applies after filter.)
          inset: "calc(var(--hm-backdrop-blur) * -3)",
          // saturate AFTER blur: blur averages colours toward grey, and this
          // restores the chroma that averaging removed. Filters apply
          // left-to-right, which is why this is inline and not Tailwind classes.
          filter: "blur(var(--hm-backdrop-blur)) saturate(1.5)",
          ...art,
        }}
      />
    </div>
  );
}

/**
 * The Dynamic theme's cover-art backdrop.
 *
 * Mounted as a negative-z child of the isolated root, so it paints above the
 * root's own background and below every piece of chrome — which then reveals it
 * purely through translucent surface tokens. Renders nothing in other themes.
 */
export default function ThemeBackdrop() {
  const resolved = useThemeStore((s) => s.resolved);
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const next = backdropSource(meta);

  // A/B ping-pong. `cover` is null for a beat after every track change while
  // tags decode, so we hold the previous art rather than flashing empty.
  const [layers, setLayers] = useState<{ a: BackdropSource | null; b: BackdropSource | null; showA: boolean }>({
    a: next, b: null, showA: true,
  });
  const lastKey = useRef(keyOf(next));

  useEffect(() => {
    const key = keyOf(next);
    if (key === lastKey.current) return;
    lastKey.current = key;
    setLayers((prev) =>
      prev.showA ? { a: prev.a, b: next, showA: false } : { a: next, b: prev.b, showA: true },
    );
  }, [next]);

  if (resolved !== "dynamic") return null;

  return (
    <div className="pointer-events-none absolute inset-0 -z-10 overflow-hidden" aria-hidden="true">
      <Layer source={layers.a} show={layers.showA} />
      <Layer source={layers.b} show={!layers.showA} />
      {/* Single darkening step. Art opacity AND a scrim would multiply, crushing
          peak white to ~31/255 and guaranteeing banding. The value lives in
          index.css as --hm-backdrop-scrim, which palette.test.ts asserts on. */}
      <div className="absolute inset-0" style={{ background: "var(--hm-backdrop-scrim)" }} />
      {/* Dither. 71 levels of blurred gradient bands on 8-bit displays. Must be
          unscaled and unblurred, or it stops working as per-pixel noise. */}
      <div className="hm-grain absolute inset-0" />
    </div>
  );
}

function keyOf(s: BackdropSource | null): string {
  if (!s) return "";
  return s.kind === "art" ? s.url : s.css;
}
```

- [ ] **Step 7: Add the grain tile**

Append to `src/styles/index.css`:

```css
/*
 * Dither for the Dynamic backdrop. The scrim leaves the art ~71 of 255 levels,
 * and a heavily blurred image is a maximally smooth gradient — it bands on
 * 8-bit displays. KDE and Windows Acrylic pair blur with noise for the same
 * reason.
 *
 * An SVG data-URI, not a live filter: the browser rasterises it once and tiles
 * it. `stitchTiles="stitch"` is load-bearing — without it the tile seams show.
 */
.hm-grain {
  pointer-events: none;
  opacity: 0.04;
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  background-size: 182px;
}
```

- [ ] **Step 8: Run the tests**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS, exit 0.

- [ ] **Step 9: Commit**

```bash
git add src/lib/reducedMotion.ts src/features/theme/ src/styles/index.css \
  src/features/player/AlbumDeck.tsx src/features/player/LyricsView.tsx
git commit -m "feat(theme): cover-art backdrop for the dynamic theme

Two crossfading art layers, a single scrim, and a grain tile. The
wrapper is promoted rather than the blurred child -- promoting the
blurred element re-blurs its texture every frame of the fade.

The art layer is oversized by 3σ rather than scaled: blur()'s length is
a standard deviation, so it bleeds ~3x that far, and transform applies
after filter so scaling magnifies the fade instead of hiding it.

Holds the previous art while nowPlayingMeta.cover is briefly null after
a track change. Promotes prefers-reduced-motion to lib/ -- this is its
third consumer."
```

---

### Task 5: Wire into the shell + kill the launch flash

**Files:**
- Modify: `src/app/App.tsx:69`
- Modify: `src/app/providers.tsx`
- Modify: `index.html`

**Interfaces:**
- Consumes: `ThemeBackdrop` (Task 4), `useThemeStore`, `watchPrefersDark`, `THEME_LIMITS` (Task 3).
- Produces: `data-theme` and `--hm-backdrop-blur` live on `<html>` from app start.

- [ ] **Step 1: Isolate the root and mount the backdrop**

`src/app/App.tsx:69` — add `relative isolate`, mount `ThemeBackdrop` as the first child:

```tsx
    <div className="relative isolate flex h-screen w-screen overflow-hidden bg-surface text-text">
      <ThemeBackdrop />
      <Sidebar />
```

with `import ThemeBackdrop from "@/features/theme/ThemeBackdrop";` at the top.

`isolate` is required, not decorative: it forces this element to be a stacking context, which is what makes the backdrop's `-z-10` paint *above the root's own background but below every non-positioned child*. Without it the backdrop escapes to an ancestor context and the chrome hides it. No other component changes — the shell reveals the art through token alpha alone.

- [ ] **Step 2: Apply the theme to `<html>`**

In `src/app/providers.tsx`, add an effect (alongside the existing ones):

```tsx
import { useThemeStore, watchPrefersDark } from "@/stores/theme";

// …inside Providers…
const resolved = useThemeStore((s) => s.resolved);
const blur = useThemeStore((s) => s.blur);
const setPrefersDark = useThemeStore((s) => s.setPrefersDark);

useEffect(() => watchPrefersDark(setPrefersDark), [setPrefersDark]);

useEffect(() => {
  const root = document.documentElement;
  root.dataset.theme = resolved;
  // The slider retargets this one variable, so dragging it never re-renders
  // the image — only the blur filter's input changes.
  root.style.setProperty("--hm-backdrop-blur", `${blur}px`);
  // The boot script painted a guess; once React owns the theme, keep them
  // agreeing so a later theme change repaints the base too.
  root.style.setProperty("--hm-boot-bg", resolved === "light" ? "#f4f5f7" : "#0a0b0e");
}, [resolved, blur]);
```

- [ ] **Step 3: Pre-paint the theme in `index.html`**

Replace the `<style>` block and add a script before `</head>`:

```html
    <!-- Paint the surface colour before CSS loads to avoid a white flash. -->
    <style>
      html,
      body {
        margin: 0;
        height: 100%;
        background: var(--hm-boot-bg, #0a0b0e);
      }
    </style>
    <script>
      // Runs before the bundle, so a Light user doesn't watch the window flash
      // near-black on every launch. Deliberately duplicates resolveTheme's rule
      // in a few lines rather than pulling the bundle forward -- keep in sync
      // with src/stores/theme.ts.
      (function () {
        try {
          var c = localStorage.getItem("hm.theme") || "system";
          if (["system", "dynamic", "light", "dark"].indexOf(c) === -1) c = "system";
          var dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
          var r = c === "system" ? (dark ? "dark" : "light") : c;
          document.documentElement.dataset.theme = r;
          document.documentElement.style.setProperty(
            "--hm-boot-bg", r === "light" ? "#f4f5f7" : "#0a0b0e"
          );
        } catch (e) {
          /* No storage — fall through to the dark default. */
        }
      })();
    </script>
```

- [ ] **Step 4: Verify it builds and runs**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/app/App.tsx src/app/providers.tsx index.html
git commit -m "feat(theme): apply the theme to the shell

Root gains `relative isolate` so the backdrop's -z-10 paints above the
root background and below every child; chrome then reveals the art
through token alpha, so no shell component changes.

index.html resolves the theme before the bundle loads -- otherwise a
light-theme user watches the window flash near-black on every launch."
```

---

### Task 6: Segmented control

**Files:**
- Create: `src/components/Segmented.tsx`

**Interfaces:**
- Consumes: `cn` from `@/lib/cn`.
- Produces: `<Segmented<T> items={{ value: T; label: string }[]} value={T} onChange={(v: T) => void} label={string} />` — generic over a string union.

There is no segmented control on this branch to reuse. (`LayoutToggle` exists only on `feat/stations-tv`; when that merges it should be refactored onto this. Do not reference it here.)

- [ ] **Step 1: Write the component**

Create `src/components/Segmented.tsx`:

```tsx
import { cn } from "@/lib/cn";

export interface SegmentedItem<T extends string> {
  value: T;
  label: string;
}

interface SegmentedProps<T extends string> {
  items: readonly SegmentedItem<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Names the group for screen readers. */
  label: string;
  className?: string;
}

/** A small exclusive choice, rendered inline rather than behind a dropdown. */
export function Segmented<T extends string>({
  items, value, onChange, label, className,
}: SegmentedProps<T>) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn(
        "flex gap-1 rounded-control border border-border bg-surface-raised p-1",
        className,
      )}
    >
      {items.map((item) => {
        const active = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(item.value)}
            className={cn(
              "flex-1 rounded-[7px] px-3 py-1.5 text-sm font-medium transition-colors",
              active ? "bg-surface-overlay text-text" : "text-text-muted hover:text-text",
            )}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 2: Verify it typechecks**

Run: `pnpm exec tsc --noEmit`
Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add src/components/Segmented.tsx
git commit -m "feat(ui): generic segmented control

An exclusive choice rendered inline. role=radiogroup + aria-checked
rather than aria-pressed: these are alternatives, not toggles."
```

---

### Task 7: Settings card

**Files:**
- Create: `src/features/settings/ThemeCard.tsx`
- Modify: `src/features/settings/SettingsView.tsx` (imports + card grid, ~:629-763)

**Interfaces:**
- Consumes: `Segmented` (Task 6); `useThemeStore`, `THEME_LIMITS` (Task 3); `Card`, `Slider` from `@/components/*`.
- Produces: `<ThemeCard />` — default export, no props.

- [ ] **Step 1: Write the card**

Create `src/features/settings/ThemeCard.tsx`:

```tsx
import { Palette } from "lucide-react";
import { Card } from "@/components/Card";
import { Slider } from "@/components/Slider";
import { Segmented } from "@/components/Segmented";
import { THEME_LIMITS, useThemeStore, type ThemeChoice } from "@/stores/theme";

const CHOICES: readonly { value: ThemeChoice; label: string }[] = [
  { value: "system", label: "System" },
  { value: "dynamic", label: "Dynamic" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export default function ThemeCard() {
  const choice = useThemeStore((s) => s.choice);
  const resolved = useThemeStore((s) => s.resolved);
  const blur = useThemeStore((s) => s.blur);
  const setChoice = useThemeStore((s) => s.setChoice);
  const setBlur = useThemeStore((s) => s.setBlur);

  const dynamic = resolved === "dynamic";

  return (
    <Card title="Appearance" icon={Palette}>
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <Segmented items={CHOICES} value={choice} onChange={setChoice} label="Theme" />
          <p className="text-xs text-text-faint">
            {choice === "system"
              ? "Follows your system appearance."
              : choice === "dynamic"
                ? "The album art of whatever's playing, blurred behind the app."
                : "Always this theme, whatever your system is set to."}
          </p>
        </div>

        <div className="flex items-center gap-3">
          <span className="w-20 shrink-0 text-sm text-text-muted">Blur</span>
          <Slider
            label="Backdrop blur"
            min={THEME_LIMITS.blur.min}
            max={THEME_LIMITS.blur.max}
            step={THEME_LIMITS.blur.step}
            value={blur}
            disabled={!dynamic}
            onChange={setBlur}
            formatValue={(v) => `${Math.round(v)} pixels`}
            // Slider needs an explicit width class: passing className replaces
            // its flex-1 default, and a 0px track silently ignores drags.
            className="flex-1"
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {Math.round(blur)}px
          </span>
        </div>
      </div>
    </Card>
  );
}
```

- [ ] **Step 2: Mount it in Settings**

In `src/features/settings/SettingsView.tsx`, add the import beside the other feature cards:

```tsx
import ThemeCard from "@/features/settings/ThemeCard";
```

and place `<ThemeCard />` in the card grid, directly after the About card and before `<MusicLibraryCard/>` — appearance is a top-level preference, not a per-source one.

- [ ] **Step 3: Verify**

Run: `pnpm test && pnpm exec tsc --noEmit && pnpm build`
Expected: PASS, exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/features/settings/ThemeCard.tsx src/features/settings/SettingsView.tsx
git commit -m "feat(settings): appearance card

Theme picker + backdrop blur. The blur slider is disabled unless the
resolved theme is Dynamic -- it has nothing to act on otherwise."
```

---

### Task 8: Reduced transparency + manual verification

**Files:**
- Modify: `src/styles/index.css`

- [ ] **Step 1: Honour `prefers-reduced-transparency`**

Append to `src/styles/index.css`:

```css
/*
 * The media query built for exactly this pattern (macOS Reduce transparency).
 * Not yet Baseline, so it is progressive enhancement: it may only ever *remove*
 * effects. Making the chrome opaque leaves the layout untouched.
 */
@media (prefers-reduced-transparency: reduce) {
  :root[data-theme="dynamic"] {
    --color-surface: #0a0b0e;
    --color-surface-raised: #14161c;
  }
  :root[data-theme="dynamic"] .hm-grain {
    display: none;
  }
}
```

The backdrop still mounts; opaque chrome simply covers it. The art remains visible behind `main`, which is the part with no surface of its own.

- [ ] **Step 2: Verify**

Run: `pnpm test && pnpm build`
Expected: PASS, exit 0.

- [ ] **Step 3: Commit**

```bash
git add src/styles/index.css
git commit -m "feat(theme): honour prefers-reduced-transparency

Opaque chrome in Dynamic when the OS asks for reduced transparency."
```

- [ ] **Step 4: Manual verification (device — the automated tests cannot cover this)**

Run `pnpm tauri dev` and confirm:

- [ ] All four themes apply, and switching is instant.
- [ ] **System** follows a live macOS appearance flip with the app open.
- [ ] **Light cold-launch shows no dark flash** (quit fully, relaunch).
- [ ] Dynamic: art appears behind sidebar/top bar/player; chrome reads as frosted.
- [ ] Blur slider moves smoothly across 8→96 with no jank.
- [ ] Track change crossfades rather than flashing empty.
- [ ] A **bright/white** cover — text stays readable everywhere.
- [ ] A cover **with text on it** — no legibility clash at default blur.
- [ ] A track with **no embedded art** — gradient matches the on-screen cover.
- [ ] No banding in the backdrop's smooth areas (the grain is doing its job).
- [ ] Dropdowns/dialogs stay opaque and readable over the art.
- [ ] macOS *Accessibility → Display → Reduce transparency* makes chrome opaque.
- [ ] Nothing playing → plain surface, no backdrop.

---

## Notes for the implementer

- **`bg-surface/NN` compounds in Dynamic.** Tailwind v4 compiles opacity modifiers to `color-mix(… var(--color-surface) NN%, transparent)`. Dynamic already gives `--color-surface` an alpha, so `bg-surface/70` becomes ~0.55 × 0.70. Check the call sites (`LyricsView.tsx:243` uses `bg-surface/70`); if any scrim looks too thin in Dynamic, give it an explicit colour rather than removing the token's alpha.
- **The scrim has exactly one home**: `--hm-backdrop-scrim` in `index.css`. The component reads it via `var()`, the test parses it from the file. Don't inline the literal anywhere.
- **WebKitGTK below 2.46 blurs slowly** (CSS filters only became Skia-backed there), and this app has been bitten before — the cross-arch audit logged WebKitGTK `shadowBlur` as a P0. If Linux drags, cap the ceiling per platform via `src/lib/platform.ts`. Don't pre-emptively add the cap.
