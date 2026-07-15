import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
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

/** Escapes regex-special characters so a literal selector can anchor a RegExp. */
function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * All custom-property declarations inside the block opened by `selector`.
 *
 * Anchored on the selector's opening brace (`selector\s*{`), not a bare
 * `indexOf(selector)`. `@theme` also appears inside the file's line-4
 * comment ("... CSS-first via @theme.") — a plain indexOf would lock onto
 * that occurrence (no brace between it and the real rule at line 8 is what
 * makes it "work" today) and silently scope every lookup to nothing the
 * moment that ordering changes.
 */
function tokens(selector: string): Record<string, string> {
  const rule = new RegExp(`${escapeRegExp(selector)}\\s*\\{`).exec(CSS);
  if (!rule) throw new Error(`palette.test: no block for ${selector}`);
  const open = rule.index + rule[0].length - 1;
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
  // Cast to a tuple: it's always exactly 2 elements, but noUncheckedIndexedAccess
  // can't see that through .sort()'s number[] return type.
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [
    number,
    number,
  ];
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

  // The brand mark (logo tile) is deliberately exempt from theming — see the
  // `--color-brand-*` comment in index.css. Asserted in all three themes even
  // though the tokens never change, so that if a `[data-theme]` override is
  // ever "helpfully" added to one of them, this starts failing (or at least
  // has the coverage to) rather than silently drifting.
  it.each(["dark", "light", "dynamic"] as const)(
    "%s: the brand mark is legible against both gradient stops",
    (name) => {
      const t = theme(name);
      expect(ratio(get(t, "--color-on-brand"), get(t, "--color-brand-from"))).toBeGreaterThanOrEqual(4.5);
      expect(ratio(get(t, "--color-on-brand"), get(t, "--color-brand-to"))).toBeGreaterThanOrEqual(4.5);
    },
  );
});

/**
 * Recursively lists `.tsx` files under `dir`. Skips (rather than throws on)
 * any entry it cannot stat or read — e.g. a permission-denied directory —
 * so one bad entry doesn't take out the whole suite. Only ever called with
 * `src/` (see `tsxFiles(src)` below), so it never walks outside the app's
 * own source.
 */
function tsxFiles(dir: string): string[] {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return [];
  }
  return entries.flatMap((entry) => {
    const full = join(dir, entry);
    let isDir: boolean;
    try {
      isDir = statSync(full).isDirectory();
    } catch {
      return [];
    }
    if (isDir) return tsxFiles(full);
    return full.endsWith(".tsx") ? [full] : [];
  });
}

describe("accent usage", () => {
  // `text-text` on an accent fill is still a live risk — --color-text is
  // near-white on dark and near-black on light, and --color-accent's fill
  // flips brightness the opposite way, so this pairing is wrong in both
  // themes. (The former `text-surface`-on-accent half of this check is now
  // covered, more strongly, by the flat ban below — see that describe block
  // for why a narrow "only inside a string literal containing bg-accent"
  // heuristic isn't a real guarantee: it can't see gradient fills or a
  // pairing that lives in a sibling class string, which is exactly how the
  // logo gradient and the MusicLibrary count badge both slipped past it.)
  it("no element pairs an accent fill with text-text", () => {
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
        if (/\btext-text(?![\w-])/.test(cls)) {
          offenders.push(`${file.replace(src, "")}: ${cls.trim().slice(0, 60)}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe("text-surface is banned outright", () => {
  // `text-surface` means "text coloured like the base surface" — which is
  // only ever a *contrasting* colour by accident (e.g. sitting on an accent
  // fill instead of on the surface itself), and it stops being one the
  // instant --color-surface flips from near-black to near-white under
  // [data-theme="light"]. The narrow heuristic above — "does this string
  // literal contain both bg-accent and text-surface" — missed the logo
  // gradient (a *from-accent* gradient fill, not `bg-accent`) and the
  // MusicLibrary count badge (`text-surface/70` in a sibling class string,
  // not the same literal as `bg-accent`). After fixing both of those there
  // is no longer a single legitimate use of this token anywhere in the app,
  // so ban the token itself, file-wide, rather than trying to special-case
  // "good" pairings again.
  //
  // The boundary matters: `text-surface-raised` and `text-surface-overlay`
  // are different, legitimate tokens and must NOT trip this. A bare `\b`
  // is not enough to exclude them — "surface" ends in a word character and
  // is followed by `-`, which is itself a non-word character, so `\b` is
  // satisfied right there regardless of what follows. The negative lookahead
  // `(?![\w-])` instead requires the next character be neither a word
  // character nor a hyphen, which rejects `-raised`/`-overlay` continuations
  // while still matching an opacity modifier (`text-surface/70`): `/` is
  // neither, so the lookahead allows it through. Verified against both cases
  // (plus `bg-surface`, `hover:text-surface`, and a mid-string occurrence)
  // with a standalone regex test before wiring it in here.
  const BANNED_TOKEN = /\btext-surface(?![\w-])/;

  it("no .tsx file under src/ contains the text-surface token", () => {
    const src = fileURLToPath(new URL("../", import.meta.url));
    const offenders: string[] = [];
    for (const file of tsxFiles(src)) {
      const body = readFileSync(file, "utf8");
      if (BANNED_TOKEN.test(body)) {
        offenders.push(file.replace(src, ""));
      }
    }
    expect(offenders).toEqual([]);
  });

  it("the ban's regex does not trip on the legitimate surface-raised/-overlay tokens", () => {
    expect(BANNED_TOKEN.test("text-surface-raised")).toBe(false);
    expect(BANNED_TOKEN.test("text-surface-overlay")).toBe(false);
  });

  it("the ban's regex still catches an opacity modifier", () => {
    expect(BANNED_TOKEN.test("text-surface/70")).toBe(true);
  });
});
