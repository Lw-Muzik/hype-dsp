import { describe, expect, it } from "vitest";
import { formatDurationLabel } from "./duration";

describe("formatDurationLabel", () => {
  it("renders under an hour as minutes only", () => {
    expect(formatDurationLabel(48 * 60)).toBe("48m");
  });

  it("renders over an hour as hours and minutes", () => {
    expect(formatDurationLabel(102 * 60)).toBe("1h 42m");
  });

  it("omits a zero-minute remainder", () => {
    expect(formatDurationLabel(120 * 60)).toBe("2h");
  });

  it("rounds to the nearest minute", () => {
    expect(formatDurationLabel(89.6 * 60)).toBe("1h 30m");
  });

  it("degrades to an empty string for invalid or non-positive input", () => {
    expect(formatDurationLabel(0)).toBe("");
    expect(formatDurationLabel(-5)).toBe("");
    expect(formatDurationLabel(NaN)).toBe("");
    expect(formatDurationLabel(Infinity)).toBe("");
  });
});
