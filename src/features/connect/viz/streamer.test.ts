import { describe, expect, it, vi } from "vitest";
import type { VizFrame } from "../protocol";
import { SOURCE_BANDS } from "./bands";
import { VizStreamer, type VizDeps, type VizOptions } from "./streamer";

const quiet = () => new Array<number>(SOURCE_BANDS).fill(0.5);
/** A bass-heavy spectrum: the lowest bands loud. */
const kick = () => quiet().map((v, k) => (k < 19 ? 0.9 : v));

function rig(options: Partial<VizOptions> = {}) {
  let now = 10_000;
  let playing = true;
  let latency = 0;
  let position = 61_400;
  let hubOffset = 1_000_000;
  const sent: Array<{ at: number; frame: VizFrame }> = [];
  let onSpectrum: ((s: readonly number[]) => void) | null = null;
  const listen = vi.fn((cb: (s: readonly number[]) => void) => {
    onSpectrum = cb;
    return stop;
  });
  const stop = vi.fn(() => {
    onSpectrum = null;
  });
  const send = vi.fn(async (frame: VizFrame) => void sent.push({ at: now, frame }));
  const deps: VizDeps = {
    listen,
    send,
    isPlaying: () => playing,
    positionMs: () => position,
    hubNowMs: () => now + hubOffset,
    nowMs: () => now,
    outputLatencyMs: () => latency,
  };
  const s = new VizStreamer(deps, options);
  const feed = (spectrum: readonly number[]) => onSpectrum?.(spectrum);
  /** Frames every `stepMs` for `totalMs` of the given spectrum. */
  const stream = (totalMs: number, stepMs = 32, spectrum: () => readonly number[] = quiet) => {
    const end = now + totalMs;
    while (now < end) {
      feed(spectrum());
      now += stepMs;
    }
  };
  return {
    s,
    sent,
    listen,
    stop,
    send,
    feed,
    stream,
    advance: (ms: number) => (now += ms),
    now: () => now,
    setPlaying: (p: boolean) => (playing = p),
    setLatency: (l: number) => (latency = l),
    setPosition: (p: number) => (position = p),
    setHubOffset: (o: number) => (hubOffset = o),
    isListening: () => onSpectrum !== null,
  };
}

describe("lifecycle: stream only while subscribed AND online AND playing", () => {
  it("does nothing until all three hold", () => {
    const r = rig();
    r.s.setOnline(true);
    expect(r.listen).not.toHaveBeenCalled(); // online + playing, nobody watching
    r.s.setSubscribed(true);
    expect(r.listen).toHaveBeenCalledOnce();
    expect(r.s.isStreaming).toBe(true);
  });

  it("does not start while paused, and starts when playback does", () => {
    const r = rig();
    r.setPlaying(false);
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    expect(r.listen).not.toHaveBeenCalled();
    r.setPlaying(true);
    r.s.refresh();
    expect(r.listen).toHaveBeenCalledOnce();
  });

  it("releases the listener when playback pauses", () => {
    const r = rig();
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    r.setPlaying(false);
    r.s.refresh();
    expect(r.stop).toHaveBeenCalledOnce();
    expect(r.s.isStreaming).toBe(false);
    r.setPlaying(true);
    r.s.refresh();
    expect(r.listen).toHaveBeenCalledTimes(2);
  });

  it("releases it on viz_sub off", () => {
    const r = rig();
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    r.s.setSubscribed(false);
    expect(r.stop).toHaveBeenCalledOnce();
  });

  it("releases it on disconnect, and a subscription does not survive its socket", () => {
    const r = rig();
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    r.s.setOnline(false);
    expect(r.stop).toHaveBeenCalledOnce();
    r.s.setOnline(true); // reconnected: nothing until the hub says viz_sub on again
    expect(r.listen).toHaveBeenCalledOnce();
    r.s.setSubscribed(true);
    expect(r.listen).toHaveBeenCalledTimes(2);
  });

  it("ignores a viz_sub on that arrives with no socket up", () => {
    const r = rig();
    r.s.setSubscribed(true);
    r.s.setOnline(true);
    expect(r.listen).not.toHaveBeenCalled();
  });

  it("does not stack listeners on repeated refreshes or repeated subscribes", () => {
    const r = rig();
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    r.s.setSubscribed(true);
    r.s.refresh();
    r.s.refresh();
    expect(r.listen).toHaveBeenCalledOnce();
  });

  it("dispose() releases everything", () => {
    const r = rig();
    r.s.setOnline(true);
    r.s.setSubscribed(true);
    r.s.dispose();
    expect(r.stop).toHaveBeenCalledOnce();
    r.s.refresh();
    expect(r.listen).toHaveBeenCalledOnce();
  });
});

function streaming(options: Partial<VizOptions> = {}) {
  const r = rig(options);
  r.s.setOnline(true);
  r.s.setSubscribed(true);
  return r;
}

describe("frame content", () => {
  it("sends the contract's shape", () => {
    const r = streaming();
    r.feed(quiet());
    const f = r.sent[0]!.frame;
    expect(f.bands).toHaveLength(32);
    expect(f.bands.every((b) => b >= 0 && b <= 1)).toBe(true);
    expect(f.level).toBe(0.5);
    expect(f.beat).toBe(0);
    expect(JSON.stringify({ t: "viz", frame: f }).length).toBeLessThan(1024);
  });

  it("stamps heardAt in HUB time and positionMs from the playback position", () => {
    const r = streaming();
    r.setHubOffset(5_000_000);
    r.setPosition(61_400);
    r.feed(quiet());
    expect(r.sent[0]!.frame.heardAt).toBe(r.now() + 5_000_000);
    expect(r.sent[0]!.frame.positionMs).toBe(61_400);
  });

  it("adds the output latency to both: the audio is heard that much later", () => {
    const r = streaming();
    r.setLatency(200);
    r.setHubOffset(1000);
    r.feed(quiet());
    expect(r.sent[0]!.frame.heardAt).toBe(r.now() + 1000 + 200);
    expect(r.sent[0]!.frame.positionMs).toBe(61_400 + 200);
  });

  it("follows the hub clock as it is re-measured", () => {
    const r = streaming();
    r.feed(quiet());
    r.advance(60);
    r.setHubOffset(1_000_250);
    r.feed(quiet());
    const [a, b] = r.sent.map((x) => x.frame.heardAt - x.at);
    expect(a).toBe(1_000_000);
    expect(b).toBe(1_000_250);
  });

  it("never sends a negative latency or position", () => {
    const r = streaming();
    r.setLatency(-500);
    r.setPosition(-20);
    r.feed(quiet());
    expect(r.sent[0]!.frame.positionMs).toBe(0);
    expect(r.sent[0]!.frame.heardAt).toBe(r.now() + 1_000_000);
  });

  it("stays quiet once playback stops even if a frame is already in flight", () => {
    const r = streaming();
    r.setPlaying(false);
    r.feed(quiet());
    expect(r.sent).toHaveLength(0);
  });
});

describe("throttle", () => {
  it("runs at ~20 fps from ~31 fps engine frames, never closer than the minimum gap", () => {
    const r = streaming();
    r.stream(2000);
    expect(r.sent.length).toBeGreaterThanOrEqual(38);
    expect(r.sent.length).toBeLessThanOrEqual(42);
    for (let i = 1; i < r.sent.length; i++) {
      expect(r.sent[i]!.at - r.sent[i - 1]!.at).toBeGreaterThanOrEqual(25);
    }
  });

  it("never exceeds the hub's 30/s in any sliding second, however fast frames arrive", () => {
    const r = streaming({ intervalMs: 1, minGapMs: 0, maxPerSecond: 30 });
    r.stream(3000, 5); // 200 engine frames a second
    const times = r.sent.map((x) => x.at);
    for (let i = 0; i < times.length; i++) {
      const inWindow = times.filter((t) => t >= times[i]! && t < times[i]! + 1000).length;
      expect(inWindow).toBeLessThanOrEqual(30);
    }
    expect(times.length).toBeGreaterThanOrEqual(85); // and it does use the budget
  });

  it("holds a custom per-second cap exactly", () => {
    const r = streaming({ intervalMs: 1, minGapMs: 0, maxPerSecond: 5 });
    r.stream(900, 10);
    expect(r.sent).toHaveLength(5);
  });

  it("a restart sends at once even right after the last frame (the old schedule is forgotten)", () => {
    const r = streaming();
    r.stream(500);
    r.advance(100);
    r.feed(quiet()); // due: goes out
    const before = r.sent.length;
    r.advance(5); // 5 ms after a send is inside the minimum gap…
    r.setPlaying(false);
    r.s.refresh();
    r.setPlaying(true);
    r.s.refresh();
    r.feed(quiet()); // …but a fresh stream owes nothing to the last one
    expect(r.sent.length).toBe(before + 1);
  });

  it("restarts the schedule cleanly after a stop and start", () => {
    const r = streaming();
    r.stream(500);
    const before = r.sent.length;
    r.setPlaying(false);
    r.s.refresh();
    r.advance(5000);
    r.setPlaying(true);
    r.s.refresh();
    r.feed(quiet());
    expect(r.sent.length).toBe(before + 1);
  });
});

describe("beat", () => {
  /** Settles on a quiet bass line for ~1 s, returning the next-frame time. */
  function settled() {
    const r = streaming();
    r.stream(1000);
    return r;
  }

  /** Settle, send one regular frame, then wait `after` ms. */
  function justSent(options: Partial<VizOptions>, after: number) {
    const r = streaming(options);
    r.stream(1000);
    r.advance(200);
    r.feed(quiet()); // due: goes out
    r.advance(after);
    return r;
  }

  it("sends a bass onset at once instead of waiting for the next slot", () => {
    // 100 ms interval, 30 ms after a send: a regular frame is not due…
    const control = justSent({ intervalMs: 100 }, 30);
    const before = control.sent.length;
    control.feed(quiet());
    expect(control.sent.length, "no onset: waits for its slot").toBe(before);

    // …but a kick at the same moment goes out immediately.
    const r = justSent({ intervalMs: 100 }, 30);
    const count = r.sent.length;
    r.feed(kick());
    expect(r.sent.length).toBe(count + 1);
    expect(r.sent.at(-1)!.frame.beat).toBeGreaterThan(0);
  });

  it("carries an onset that falls inside the minimum gap to the next frame it sends", () => {
    const r = justSent({}, 10); // 10 ms after a send: inside the 25 ms gap
    const count = r.sent.length;
    r.feed(kick());
    expect(r.sent.length, "too soon to send").toBe(count);
    r.advance(40);
    r.feed(kick());
    expect(r.sent.length).toBe(count + 1);
    expect(r.sent.at(-1)!.frame.beat).toBeGreaterThan(0);
  });

  it("reports beat 0 on frames without an onset, and clears it once sent", () => {
    const r = settled();
    r.advance(50);
    r.feed(kick());
    r.advance(60);
    r.feed(kick()); // the kick is sustained, not a new onset
    expect(r.sent.at(-1)!.frame.beat).toBe(0);
  });

  it("never reports a beat outside 0..1", () => {
    const r = settled();
    r.advance(50);
    r.feed(kick());
    for (const { frame } of r.sent) {
      expect(frame.beat).toBeGreaterThanOrEqual(0);
      expect(frame.beat).toBeLessThanOrEqual(1);
    }
  });

  it("starts the detector over when the track changes", () => {
    const r = settled();
    r.s.resetAnalysis();
    r.advance(50);
    const count = r.sent.length;
    r.feed(kick()); // inside the warm-up after a reset: not a beat
    expect(r.sent.at(-1)!.frame.beat).toBe(0);
    expect(r.sent.length).toBe(count + 1);
  });
});

describe("failures", () => {
  it("a rejected send does not throw or stop the stream", async () => {
    const r = streaming();
    r.send.mockRejectedValueOnce(new Error("offline"));
    expect(() => r.stream(300)).not.toThrow();
    await Promise.resolve();
    expect(r.send.mock.calls.length).toBeGreaterThan(1);
    expect(r.s.isStreaming).toBe(true);
  });
});
