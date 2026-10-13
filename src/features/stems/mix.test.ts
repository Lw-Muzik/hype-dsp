import { describe, expect, it } from "vitest";
import { effectiveGains, initialControls, isOriginal } from "./mix";

describe("effectiveGains", () => {
  it("is unity for every control by default (the original track)", () => {
    expect(effectiveGains(initialControls())).toEqual([1, 1, 1]);
    expect(isOriginal(initialControls())).toBe(true);
  });

  it("silences a muted control and leaves the others", () => {
    const c = initialControls();
    c.vocals.muted = true;
    expect(effectiveGains(c)).toEqual([0, 1, 1]);
    expect(isOriginal(c)).toBe(false);
  });

  it("follows the fader", () => {
    const c = initialControls();
    c.drums.level = 0.4;
    expect(effectiveGains(c)).toEqual([1, 0.4, 1]);
  });

  it("solo silences everything that isn't soloed", () => {
    const c = initialControls();
    c.drums.solo = true;
    expect(effectiveGains(c)).toEqual([0, 1, 0]);
  });

  it("two solos play together, at their own levels", () => {
    const c = initialControls();
    c.vocals.solo = true;
    c.instruments.solo = true;
    c.instruments.level = 0.5;
    expect(effectiveGains(c)).toEqual([1, 0, 0.5]);
  });

  it("solo beats mute on the same control", () => {
    const c = initialControls();
    c.vocals.solo = true;
    c.vocals.muted = true;
    expect(effectiveGains(c)).toEqual([1, 0, 0]);
  });
});
