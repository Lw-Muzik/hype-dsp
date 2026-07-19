import { describe, expect, it } from "vitest";
import { seekDisplayValue } from "./SeekBar";

describe("seekDisplayValue", () => {
  /** The rule the whole scrub pattern exists for: while dragging, the thumb
   *  follows the finger, not the still-advancing playback position. */
  it("shows the scrub value while dragging, ignoring playback position", () => {
    expect(seekDisplayValue(30, 12, 200)).toBe(30);
  });

  it("shows the real position when not dragging", () => {
    expect(seekDisplayValue(null, 12, 200)).toBe(12);
  });

  /** A position past a known duration (rounding, a stream over-reporting) must
   *  not push the thumb off the end of the bar. */
  it("clamps the resting position to the duration", () => {
    expect(seekDisplayValue(null, 205, 200)).toBe(200);
  });

  /** Before a stream's length is known, there's nothing to clamp to; show the
   *  position as-is rather than snapping it to zero. */
  it("shows the position unclamped when duration is unknown", () => {
    expect(seekDisplayValue(null, 42, 0)).toBe(42);
  });

  /** Scrubbing to zero is a real value, not "no scrub" — it must not fall
   *  through to the playback position. */
  it("treats a scrub value of 0 as an active scrub", () => {
    expect(seekDisplayValue(0, 55, 200)).toBe(0);
  });
});
