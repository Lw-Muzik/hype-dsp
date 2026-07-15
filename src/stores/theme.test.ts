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
