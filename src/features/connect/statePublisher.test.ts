import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PlayerState } from "./protocol";
import {
  buildPlayerState,
  createStatePublisher,
  DEBOUNCE_MS,
  firstSeq,
  HEARTBEAT_MS,
  localStorageSeqStore,
  MAX_STATE_QUEUE,
  type EngineView,
  type SeqStore,
  type StateBody,
} from "./statePublisher";
import { QueueUidTracker } from "./queueUids";
import { DEVICE, localQ, ytItem } from "./testKit";

const engine = (over: Partial<EngineView> = {}): EngineView => ({
  playing: false,
  paused: false,
  nowPlaying: null,
  positionSecs: 0,
  durationSecs: null,
  queue: [],
  queueIndex: -1,
  order: [],
  orderPos: -1,
  shuffle: false,
  repeat: "off",
  state: { masterVolume: 0.8 },
  ...over,
});

describe("buildPlayerState", () => {
  it("is idle with no track when nothing is loaded", () => {
    const s = buildPlayerState(engine(), DEVICE, new QueueUidTracker());
    expect(s).toMatchObject({ track: null, playing: false, positionMs: 0, durationMs: null, queue: [] });
  });

  it("a queue parked on its last item after a natural end is idle, not paused", () => {
    const q = [ytItem("a")];
    const s = buildPlayerState(
      engine({ queue: q, queueIndex: 0, order: [0], orderPos: 0, playing: false, nowPlaying: null }),
      DEVICE,
      new QueueUidTracker(),
    );
    expect(s.track).toBeNull();
    expect(s.queue).toEqual([]);
  });

  it("reports the track, position in ms, upcoming entries in play order, and volume clamped", () => {
    const q = [ytItem("a"), ytItem("b"), localQ("/c.mp3")];
    const uids = new QueueUidTracker();
    const ids = uids.uidsFor(q);
    const s = buildPlayerState(
      engine({
        playing: true,
        nowPlaying: "Song a",
        positionSecs: 12.3456,
        durationSecs: 200,
        queue: q,
        queueIndex: 0,
        order: [0, 2, 1],
        orderPos: 0,
        shuffle: true,
        repeat: "all",
        state: { masterVolume: 1.4 },
      }),
      DEVICE,
      uids,
    );
    expect(s.playing).toBe(true);
    expect(s.positionMs).toBe(12_346);
    expect(s.durationMs).toBe(200_000);
    expect(s.volume).toBe(1);
    expect(s.shuffle).toBe(true);
    expect(s.repeat).toBe("all");
    expect(s.track).toMatchObject({ src: "yt", videoId: "a" });
    expect(s.queue.map((e) => e.uid)).toEqual([ids[2], ids[1]]);
    expect(s.queue[0]!.track).toMatchObject({ src: "local", deviceId: DEVICE, id: "/c.mp3" });
  });

  it("paused is not playing; falls back to the item's duration", () => {
    const q = [ytItem("a")];
    const s = buildPlayerState(
      engine({ playing: true, paused: true, nowPlaying: "x", durationSecs: null, queue: q, queueIndex: 0, order: [0], orderPos: 0 }),
      DEVICE,
      new QueueUidTracker(),
    );
    expect(s.playing).toBe(false);
    expect(s.durationMs).toBe(200_000);
  });

  it("caps the upcoming queue at 50", () => {
    const q = Array.from({ length: 80 }, (_, i) => ytItem(`t${i}`));
    const s = buildPlayerState(
      engine({ playing: true, nowPlaying: "x", queue: q, queueIndex: 0, order: q.map((_, i) => i), orderPos: 0 }),
      DEVICE,
      new QueueUidTracker(),
    );
    expect(s.queue).toHaveLength(MAX_STATE_QUEUE);
    expect(MAX_STATE_QUEUE).toBe(50);
  });
});

describe("firstSeq", () => {
  it("is the epoch in seconds when nothing is persisted", () => {
    expect(firstSeq(1_760_000_000_999, null)).toBe(1_760_000_000);
  });
  it("is one past the persisted value when that is ahead of the clock", () => {
    expect(firstSeq(1_760_000_000_000, 1_760_000_500)).toBe(1_760_000_501);
  });
  it("is the clock when it is ahead of the persisted value", () => {
    expect(firstSeq(1_760_000_100_000, 1_760_000_000)).toBe(1_760_000_100);
  });
  it("stays within 32-bit range for today's dates", () => {
    expect(firstSeq(Date.now(), null)).toBeLessThan(2 ** 31);
  });
  it("ignores a non-finite persisted value", () => {
    expect(firstSeq(5_000, Number.NaN)).toBe(5);
  });
});

describe("localStorageSeqStore", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("round-trips through localStorage", () => {
    const mem = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => mem.get(k) ?? null,
      setItem: (k: string, v: string) => void mem.set(k, v),
    });
    const store = localStorageSeqStore();
    expect(store.load()).toBeNull();
    store.save(1_760_000_123);
    expect(store.load()).toBe(1_760_000_123);
  });

  it("treats garbage as absent", () => {
    vi.stubGlobal("localStorage", { getItem: () => "abc", setItem: () => {} });
    expect(localStorageSeqStore().load()).toBeNull();
  });

  it("never throws when storage is blocked or missing", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    });
    const store = localStorageSeqStore();
    expect(store.load()).toBeNull();
    expect(() => store.save(1)).not.toThrow();
    vi.stubGlobal("localStorage", undefined);
    expect(localStorageSeqStore().load()).toBeNull();
  });
});

describe("createStatePublisher", () => {
  let body: StateBody;
  let listener: (() => void) | null;
  let sent: PlayerState[];
  let seqStore: SeqStore & { value: number | null };

  const idle = (): StateBody => ({
    track: null,
    playing: false,
    positionMs: 0,
    durationMs: null,
    volume: 0.5,
    shuffle: false,
    repeat: "off",
    queue: [],
  });
  const track = ytItem("a");
  const playing = (positionMs: number): StateBody => ({
    ...idle(),
    track: { src: "yt", videoId: "a", title: "A", artist: null, album: null, durationMs: 200_000, artUrl: null },
    playing: true,
    positionMs,
    durationMs: 200_000,
  });

  const make = () =>
    createStatePublisher({
      build: () => body,
      subscribe: (cb) => {
        listener = cb;
        return () => {
          listener = null;
        };
      },
      send: async (s) => void sent.push(s),
      now: () => Date.now(),
      seqStore,
    });
  const change = () => listener!();

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-12T12:00:00Z"));
    body = idle();
    listener = null;
    sent = [];
    seqStore = {
      value: null,
      load() {
        return this.value;
      },
      save(n) {
        this.value = n;
      },
    };
    void track;
  });
  afterEach(() => vi.useRealTimers());

  it("sends immediately on start with seq = epoch seconds, then increments and persists", () => {
    const p = make();
    p.start();
    const epochSecs = Math.floor(Date.now() / 1000);
    expect(sent.map((s) => s.seq)).toEqual([epochSecs]);
    p.publishNow();
    p.publishNow();
    expect(sent.map((s) => s.seq)).toEqual([epochSecs, epochSecs + 1, epochSecs + 2]);
    expect(seqStore.value).toBe(epochSecs + 2);
  });

  it("a restart continues above the persisted seq even if the clock is behind it", () => {
    seqStore.value = Math.floor(Date.now() / 1000) + 10_000;
    const p = make();
    p.start();
    expect(sent[0]!.seq).toBe(seqStore.value);
    expect(sent[0]!.seq).toBe(Math.floor(Date.now() / 1000) + 10_001);
  });

  it("seq stays monotonic across stop/start within one run", () => {
    const p = make();
    p.start();
    p.stop();
    p.start();
    expect(sent[1]!.seq).toBe(sent[0]!.seq + 1);
  });

  it("coalesces a burst of changes into one send after 300 ms", () => {
    const p = make();
    p.start();
    sent.length = 0;
    body = playing(0);
    change();
    body = { ...playing(0), volume: 0.9 };
    change();
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS - 1);
    expect(sent).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.volume).toBe(0.9); // the latest state, not the first
  });

  it("ignores ticking position alone", () => {
    const p = make();
    body = playing(0);
    p.start();
    sent.length = 0;
    vi.advanceTimersByTime(8000);
    body = playing(8000); // exactly where playback should be
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS * 2);
    expect(sent).toHaveLength(0);
  });

  it("detects a seek: a jump beyond 2 s of where playback should be", () => {
    const p = make();
    body = playing(0);
    p.start();
    sent.length = 0;
    vi.advanceTimersByTime(1000);
    body = playing(60_000);
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.positionMs).toBe(60_000);
  });

  it("detects a backward seek too", () => {
    const p = make();
    body = playing(100_000);
    p.start();
    sent.length = 0;
    body = playing(5_000);
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(1);
  });

  it("a paused position that does not advance is not a seek, but moving it is", () => {
    const p = make();
    body = { ...playing(30_000), playing: false };
    p.start();
    sent.length = 0;
    vi.advanceTimersByTime(10_000);
    body = { ...playing(30_000), playing: false };
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(0);
    body = { ...playing(90_000), playing: false };
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(1);
  });

  it("sends on play/pause and on a new queue", () => {
    const p = make();
    body = playing(0);
    p.start();
    sent.length = 0;
    body = { ...playing(0), playing: false };
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(1);
    body = { ...body, queue: [{ uid: "q-9", track: body.track! }] };
    change();
    vi.advanceTimersByTime(DEBOUNCE_MS);
    expect(sent).toHaveLength(2);
  });

  it("heartbeats every 15 s while playing, and not while paused", () => {
    const p = make();
    body = playing(0);
    p.start();
    sent.length = 0;
    vi.advanceTimersByTime(HEARTBEAT_MS);
    expect(sent).toHaveLength(1);
    vi.advanceTimersByTime(HEARTBEAT_MS);
    expect(sent).toHaveLength(2);
    body = { ...playing(0), playing: false };
    p.publishNow();
    sent.length = 0;
    vi.advanceTimersByTime(HEARTBEAT_MS * 4);
    expect(sent).toHaveLength(0);
  });

  it("stop cancels pending sends and heartbeats and unsubscribes", () => {
    const p = make();
    body = playing(0);
    p.start();
    body = { ...playing(0), volume: 0.1 };
    change();
    p.stop();
    sent.length = 0;
    vi.advanceTimersByTime(HEARTBEAT_MS * 3);
    expect(sent).toHaveLength(0);
    expect(listener).toBeNull();
    p.publishNow(); // not running: no-op
    expect(sent).toHaveLength(0);
  });

  it("a failing send does not break publishing", async () => {
    const p = createStatePublisher({
      build: () => body,
      subscribe: () => () => {},
      send: () => Promise.reject(new Error("socket down")),
      now: () => Date.now(),
    });
    expect(() => p.start()).not.toThrow();
    await vi.advanceTimersByTimeAsync(10);
  });
});
