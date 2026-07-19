import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Text fields must show focus.
 *
 * `index.css` used to guarantee this for free: a blanket `:focus-visible`
 * outline covered every control, styled or not. It also drew a *second* ring
 * around fields that already coloured a border on focus, so a focused search box
 * wore two — one hugging the field, one around its row. The blanket rule now
 * excludes text fields.
 *
 * That removed a safety net. A text input added without its own focus style used
 * to be quietly covered; now it would simply be invisible to a keyboard user,
 * and nothing about writing it would look wrong. This is the replacement: the
 * app's convention — every text field, or the wrapper it sits in, colours a
 * border or ring on focus — enforced instead of assumed.
 *
 * Granularity is per file, matching `palette.test.ts`. It catches the realistic
 * regression (an input in a file that styles focus nowhere) and not a second
 * unstyled input in a file that already has one. Worth stating rather than
 * implying: this is a tripwire, not a proof.
 */
const SRC = fileURLToPath(new URL("../", import.meta.url));

/** A text-entry field. Range/checkbox/radio are excluded because they keep the
 *  blanket outline — a slider has no border to colour, so it is their only
 *  focus indicator. */
const TEXT_FIELD = /<(input|textarea)\b/;
const NON_TEXT_INPUT = /type=["'](range|checkbox|radio|hidden)["']/;

/** Any focus treatment the app uses: on the field, or on a `focus-within`
 *  wrapper. Covers `focus:`, `focus-visible:` and `focus-within:`. */
const FOCUS_STYLE = /focus(-within|-visible)?:(border|ring|outline-)/;

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

describe("text fields keep a visible focus indicator", () => {
  it("every file with a text input styles focus somewhere in it", () => {
    const offenders: string[] = [];

    for (const file of tsxFiles(SRC)) {
      const body = readFileSync(file, "utf8");
      if (!TEXT_FIELD.test(body)) continue;

      // A file whose only fields are sliders/checkboxes still relies on the
      // blanket outline, which is correct — nothing to enforce here.
      const hasTextField = body
        .split(/<(?=input\b|textarea\b)/)
        .slice(1)
        .some((tag) => !NON_TEXT_INPUT.test(tag.slice(0, 400)));
      if (!hasTextField) continue;

      if (!FOCUS_STYLE.test(body)) offenders.push(file.replace(SRC, ""));
    }

    expect(offenders).toEqual([]);
  });

  /// The rule this test exists to protect. If the exclusion is ever dropped from
  /// index.css the double ring comes back, and every assertion above still
  /// passes — so check the stylesheet itself.
  it("index.css exempts text fields from the blanket focus outline", () => {
    const css = readFileSync(join(SRC, "styles/index.css"), "utf8");
    expect(css).toMatch(/input:not\(\[type="range"\]\)[^{]*:focus-visible/);
    expect(css).toMatch(/textarea:focus-visible/);
  });
});
