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
});
