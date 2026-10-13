import { describe, expect, it } from "vitest";
import { BassBeat, type BassHit, MIN_STRENGTH } from "./bassBeat";

const DT = 1 / 30;

/** Feed `dbs` one per frame; returns [frame index, hit] for every report. */
function run(beat: BassBeat, dbs: number[], dt = DT): Array<[number, BassHit]> {
  const out: Array<[number, BassHit]> = [];
  dbs.forEach((db, i) => {
    const hit = beat.step(db, dt);
    if (hit) out.push([i, hit]);
  });
  return out;
}

const steady = (db: number, n: number) => new Array<number>(n).fill(db);

describe("BassBeat", () => {
  it("is quiet on a steady bass line", () => {
    expect(run(new BassBeat(), steady(-30, 120))).toEqual([]);
  });

  it("fires on a sharp rise out of a settled level, on the frame it happens", () => {
    const hits = run(new BassBeat(), [...steady(-30, 30), -16, -16, -16]);
    expect(hits).toHaveLength(1);
    const [frame, hit] = hits[0]!;
    expect(frame).toBe(30);
    expect(hit.onset).toBe(true);
    expect(hit.strength).toBeGreaterThan(MIN_STRENGTH);
    expect(hit.strength).toBeLessThanOrEqual(1);
    expect(hit.gain).toBe(hit.strength);
  });

  it("is silent through the warm-up after a reset (the baseline starts from nothing)", () => {
    expect(run(new BassBeat(), [-60, -20, -20, -20])).toEqual([]);
  });

  it("ignores a rise that is slow, even if it climbs far", () => {
    const swell = Array.from({ length: 60 }, (_, i) => -40 + i * 0.5); // +0.5 dB / frame
    expect(run(new BassBeat(), [...steady(-40, 20), ...swell])).toEqual([]);
  });

  it("ignores a rise that stays under the absolute floor", () => {
    expect(run(new BassBeat(), [...steady(-75, 30), -62, -62])).toEqual([]);
  });

  it("ignores a hit far quieter than the song's recent loudest bass", () => {
    // 30 dB of headroom below a loud passage: a 4 dB bump there is not a hit.
    const frames = [...steady(-10, 15), ...steady(-40, 30), -34, -34];
    expect(run(new BassBeat(), frames).filter(([i]) => i >= 45)).toEqual([]);
  });

  it("tops a hit up while its attack keeps rising, then reports nothing more", () => {
    // A loud passage first, so the hit is judged against a higher recent peak
    // (a hit that sets its own peak is already at full strength).
    const hits = run(new BassBeat(), [...steady(-20, 15), ...steady(-42, 40), -34, -30, -30, -30]);
    expect(hits[0]![1].onset).toBe(true);
    const top = hits.find(([, h]) => !h.onset);
    expect(top, "the rising second frame tops the hit up").toBeDefined();
    expect(top![1].gain).toBeGreaterThan(0);
    expect(top![1].strength).toBeGreaterThan(hits[0]![1].strength);
    expect(hits).toHaveLength(2);
  });

  it("does not fire twice inside the refractory period, even out of a deep notch", () => {
    // Hit, a deep notch (so the baseline falls and the second rise is huge),
    // then another rise 66 ms after the first.
    const frames = [...steady(-30, 30), -18, -45, -18, -18];
    const onsets = run(new BassBeat(), frames).filter(([, h]) => h.onset);
    expect(onsets.map(([i]) => i)).toEqual([30]);
  });

  it("fires again once the refractory period has passed", () => {
    const frames = [...steady(-30, 30), -18, ...steady(-45, 6), -18, -18];
    const onsets = run(new BassBeat(), frames).filter(([, h]) => h.onset);
    expect(onsets.map(([i]) => i)).toEqual([30, 37]);
  });

  it("does not re-arm on a hit's own tail (the baseline starts just under its peak)", () => {
    // After a hit the level settles a few dB lower and wobbles: not new hits.
    const frames = [...steady(-30, 30), -16, -19, -22, -19, -22, -19, -22, -19];
    const onsets = run(new BassBeat(), frames).filter(([, h]) => h.onset);
    expect(onsets).toHaveLength(1);
  });

  it("forgets the song on reset()", () => {
    const beat = new BassBeat();
    run(beat, [...steady(-30, 30), -16]);
    beat.reset();
    // Straight after a reset the same jump is inside the warm-up: no hit.
    expect(run(beat, [-30, -30, -16])).toEqual([]);
    // …and a settled level followed by a jump fires again.
    expect(run(beat, [...steady(-30, 30), -16]).filter(([, h]) => h.onset)).toHaveLength(1);
  });

  it("survives junk input", () => {
    const beat = new BassBeat();
    expect(beat.step(Number.NaN, DT)).toBeNull();
    expect(beat.step(Number.NEGATIVE_INFINITY, Number.NaN)).toBeNull();
    expect(beat.step(-30, -1)).toBeNull();
  });
});
