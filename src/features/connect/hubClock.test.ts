import { describe, expect, it, vi } from "vitest";
import {
  CLOCK_INTERVAL_MS,
  CLOCK_WINDOW,
  ClockSync,
  HubClock,
  MAX_ROUND_TRIP_MS,
} from "./hubClock";

describe("HubClock", () => {
  it("measures offset = serverTime - (sent + received) / 2", () => {
    const c = new HubClock();
    // Hub clock is 5000 ms ahead; the pong took 100 ms, answered mid-flight.
    expect(c.addSample(1000, 1100, 1050 + 5000)).toBe(true);
    expect(c.offsetMs).toBe(5000);
    expect(c.roundTripMs).toBe(100);
  });

  it("is unknown before any sample, and nowMs() is then the local clock", () => {
    const c = new HubClock(() => 123_456);
    expect(c.offsetMs).toBeNull();
    expect(c.roundTripMs).toBeNull();
    expect(c.isMeasured).toBe(false);
    expect(c.nowMs()).toBe(123_456);
  });

  it("nowMs() is local time plus the offset", () => {
    let local = 10_000;
    const c = new HubClock(() => local);
    c.addSample(0, 40, 20 + 700);
    expect(c.nowMs()).toBe(10_700);
    local += 250;
    expect(c.nowMs()).toBe(10_950);
  });

  it("trusts the fastest round trip, not the newest", () => {
    const c = new HubClock();
    c.addSample(0, 400, 200 + 1000); // slow: offset 1000, rtt 400
    c.addSample(1000, 1040, 1020 + 1300); // fast: offset 1300, rtt 40
    c.addSample(2000, 2300, 2150 + 900); // slow again
    expect(c.offsetMs).toBe(1300);
    expect(c.roundTripMs).toBe(40);
  });

  it("on a tie, the newest sample wins", () => {
    const c = new HubClock();
    c.addSample(0, 50, 25 + 100);
    c.addSample(100, 150, 125 + 200);
    expect(c.offsetMs).toBe(200);
  });

  it("lets an old good sample age out after the window, so the offset can follow a drifting clock", () => {
    const c = new HubClock();
    c.addSample(0, 10, 5 + 1000); // the best ever, offset 1000
    for (let i = 0; i < 5; i++) {
      c.addSample(i * 1000, i * 1000 + 100, i * 1000 + 50 + 2000); // offset 2000, rtt 100
    }
    expect(c.offsetMs).toBe(2000);
  });

  it("compares the last five samples by default, every ten seconds", () => {
    expect(CLOCK_WINDOW).toBe(5);
    expect(CLOCK_INTERVAL_MS).toBe(10_000);
  });

  it("keeps exactly the last `window` samples", () => {
    const c = new HubClock(() => 0, 2);
    c.addSample(0, 10, 5 + 111); // rtt 10, will be pushed out
    c.addSample(0, 20, 10 + 222);
    c.addSample(0, 30, 15 + 333);
    expect(c.offsetMs).toBe(222); // 111 gone, fastest of the last two
  });

  it("rejects a negative or absurd round trip, and non-finite input", () => {
    const c = new HubClock();
    expect(c.addSample(1000, 900, 5000)).toBe(false); // clock stepped back
    expect(c.addSample(0, MAX_ROUND_TRIP_MS + 1, 5000)).toBe(false);
    expect(c.addSample(0, Number.NaN, 5000)).toBe(false);
    expect(c.addSample(0, 10, Number.NaN)).toBe(false);
    expect(c.offsetMs).toBeNull();
    expect(c.addSample(0, MAX_ROUND_TRIP_MS, 5000)).toBe(true);
  });

  it("uses the welcome's time only until a real sample arrives", () => {
    const c = new HubClock(() => 0);
    c.seedFromWelcome(9_000, 8_000);
    expect(c.offsetMs).toBe(1000);
    expect(c.isMeasured, "a seed is not a measurement").toBe(false);
    c.seedFromWelcome(99_000, 8_000); // only the first seed counts
    expect(c.offsetMs).toBe(1000);
    c.addSample(0, 20, 10 + 400);
    expect(c.offsetMs).toBe(400);
    expect(c.isMeasured).toBe(true);
  });

  it("clear() forgets samples and seed", () => {
    const c = new HubClock();
    c.seedFromWelcome(2, 1);
    c.addSample(0, 10, 100);
    c.clear();
    expect(c.offsetMs).toBeNull();
  });
});

function sync() {
  let now = 1_000_000;
  const sent: string[] = [];
  let tick: (() => void) | null = null;
  const cleared: unknown[] = [];
  const clock = new HubClock(() => now);
  const s = new ClockSync({
    clock,
    sendPing: vi.fn(async (id: string) => void sent.push(id)),
    now: () => now,
    setInterval: (fn, ms) => {
      expect(ms).toBe(10_000);
      tick = fn;
      return "timer";
    },
    clearInterval: (h) => void cleared.push(h),
  });
  return {
    s,
    clock,
    sent,
    cleared,
    advance: (ms: number) => (now += ms),
    fire: () => tick?.(),
    hasTimer: () => tick !== null,
  };
}

describe("ClockSync", () => {
  it("pings at once on start, then on every interval tick", () => {
    const t = sync();
    t.s.start();
    expect(t.sent).toHaveLength(1);
    t.fire();
    t.fire();
    expect(t.sent).toHaveLength(3);
    expect(new Set(t.sent).size, "ids are unique").toBe(3);
  });

  it("turns a pong into a sample against the time the ping left", () => {
    const t = sync();
    t.s.start();
    t.advance(80);
    t.s.onPong(t.sent[0]!, 1_000_040 + 700); // hub is 700 ahead
    expect(t.clock.offsetMs).toBe(700);
    expect(t.clock.roundTripMs).toBe(80);
  });

  it("ignores a pong it never asked for, and a repeated one", () => {
    const t = sync();
    t.s.start();
    t.s.onPong("someone-else", 5);
    expect(t.clock.isMeasured).toBe(false);
    t.advance(10);
    t.s.onPong(t.sent[0]!, 1_000_005 + 1);
    t.advance(500);
    t.s.onPong(t.sent[0]!, 9_999_999);
    expect(t.clock.offsetMs).toBe(1);
  });

  it("stop() cancels the timer and forgets in-flight pings, but keeps the measured offset", () => {
    const t = sync();
    t.s.start();
    t.advance(20);
    t.s.onPong(t.sent[0]!, 1_000_010 + 50);
    t.fire();
    const inFlight = t.sent[1]!;
    t.s.stop();
    expect(t.cleared).toEqual(["timer"]);
    t.s.onPong(inFlight, 0);
    expect(t.clock.offsetMs).toBe(50);
  });

  it("start() again (a new welcome) replaces the timer instead of stacking one", () => {
    const t = sync();
    t.s.start();
    t.s.start();
    expect(t.cleared).toEqual(["timer"]);
    expect(t.sent).toHaveLength(2);
  });

  it("forgets a ping that could not be sent", async () => {
    let now = 5;
    const clock = new HubClock(() => now);
    const ids: string[] = [];
    const s = new ClockSync({
      clock,
      sendPing: async (id) => {
        ids.push(id);
        throw new Error("offline");
      },
      now: () => now,
      setInterval: () => 1,
      clearInterval: () => {},
    });
    s.start();
    await Promise.resolve();
    await Promise.resolve();
    now = 50;
    s.onPong(ids[0]!, 100);
    expect(clock.isMeasured).toBe(false);
  });
});
