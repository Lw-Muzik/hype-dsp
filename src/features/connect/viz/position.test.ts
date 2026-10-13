import { describe, expect, it } from "vitest";
import { PositionExtrapolator } from "./position";

describe("PositionExtrapolator", () => {
  it("advances the last report by the time since it, while running", () => {
    const p = new PositionExtrapolator();
    p.sync(61_400, 1000, true);
    expect(p.positionAt(1000)).toBe(61_400);
    expect(p.positionAt(1060)).toBe(61_460);
  });

  it("holds still while paused or buffering", () => {
    const p = new PositionExtrapolator();
    p.sync(5000, 1000, false);
    expect(p.positionAt(9000)).toBe(5000);
  });

  it("never runs backwards if the clock reading precedes the report", () => {
    const p = new PositionExtrapolator();
    p.sync(5000, 1000, true);
    expect(p.positionAt(900)).toBe(5000);
  });

  it("re-anchors on every report and tolerates junk", () => {
    const p = new PositionExtrapolator();
    p.sync(1000, 0, true);
    p.sync(2000, 500, true);
    expect(p.positionAt(600)).toBe(2100);
    p.sync(Number.NaN, 0, true);
    expect(p.positionAt(0)).toBe(0);
    p.sync(-50, 0, false);
    expect(p.positionAt(10)).toBe(0);
  });
});
