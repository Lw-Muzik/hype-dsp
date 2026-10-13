import { describe, expect, it } from "vitest";
import { localItem, type QueueItem } from "@/stores/engine";
import {
  MAX_SESSION_TRACKS,
  SESSION_TRACKS_BEHIND,
  parseSession,
  snapshotSession,
  type SessionSource,
} from "./playbackSession";

const local = (n: number, over: Partial<QueueItem> = {}): QueueItem => ({
  ...localItem({ path: `/m/${n}.mp3`, title: `T${n}`, artist: null, album: null, genre: null, durationSecs: 200 }),
  ...over,
});

const source = (over: Partial<SessionSource> = {}): SessionSource => ({
  queue: [local(0), local(1), local(2)],
  order: [0, 1, 2],
  orderPos: 1,
  shuffle: false,
  repeat: "off",
  positionSecs: 42,
  resumeAt: null,
  ...over,
});

describe("snapshotSession", () => {
  it("captures the queue, order, current track and position", () => {
    const snap = snapshotSession(source());
    expect(snap).toMatchObject({ v: 1, order: [0, 1, 2], orderPos: 1, shuffle: false, repeat: "off", positionSecs: 42 });
    expect(snap?.queue.map((q) => q.id)).toEqual(["/m/0.mp3", "/m/1.mp3", "/m/2.mp3"]);
  });

  it("has nothing to save for an empty queue or a phone cast", () => {
    expect(snapshotSession(source({ queue: [], order: [], orderPos: -1 }))).toBeNull();
    const cast: QueueItem = { id: "x", source: "cast", title: "Cast", artist: null, album: null, durationSecs: null };
    expect(snapshotSession(source({ queue: [cast], order: [0], orderPos: 0 }))).toBeNull();
  });

  it("keeps a restored track's saved position until it is played", () => {
    expect(snapshotSession(source({ positionSecs: 0, resumeAt: 95 }))?.positionSecs).toBe(95);
  });

  it("stores no position for a live station", () => {
    // Built the way `playRadio` builds a station's item.
    const station: QueueItem = {
      id: "https://fm/stream",
      source: "radio",
      title: "FM",
      artist: null,
      album: null,
      durationSecs: null,
      radioUrl: "https://fm/stream",
    };
    expect(snapshotSession(source({ queue: [station], order: [0], orderPos: 0, positionSecs: 300 }))?.positionSecs).toBe(0);
  });

  it("drops inline cover art but keeps a cover URL", () => {
    const snap = snapshotSession(
      source({
        queue: [local(0, { cover: "data:image/png;base64,AAAA" }), local(1, { cover: "https://i/c.jpg" }), local(2)],
      }),
    );
    expect(snap?.queue[0]).not.toHaveProperty("cover");
    expect(snap?.queue[1]?.cover).toBe("https://i/c.jpg");
  });

  it("windows a long queue around the current track, in play order", () => {
    const n = MAX_SESSION_TRACKS * 4;
    const queue = Array.from({ length: n }, (_, i) => local(i));
    // Reversed play order, to prove the window follows play order.
    const order = queue.map((_, i) => n - 1 - i);
    const snap = snapshotSession(source({ queue, order, orderPos: 1000 }))!;
    expect(snap.queue).toHaveLength(MAX_SESSION_TRACKS);
    expect(snap.order).toEqual(snap.queue.map((_, i) => i));
    expect(snap.orderPos).toBe(SESSION_TRACKS_BEHIND);
    // The current track is the same one.
    expect(snap.queue[snap.orderPos]?.id).toBe(queue[order[1000]!]!.id);
    // And the one after it is still next.
    expect(snap.queue[snap.orderPos + 1]?.id).toBe(queue[order[1001]!]!.id);
  });

  it("windows from the start or the end when the track is near either (keeping the most)", () => {
    const n = MAX_SESSION_TRACKS + 100;
    const queue = Array.from({ length: n }, (_, i) => local(i));
    const order = queue.map((_, i) => i);
    expect(snapshotSession(source({ queue, order, orderPos: 10 }))!.orderPos).toBe(10);
    const end = snapshotSession(source({ queue, order, orderPos: n - 10 }))!;
    expect(end.orderPos).toBe(MAX_SESSION_TRACKS - 10);
    expect(end.queue[end.orderPos]?.id).toBe(`/m/${n - 10}.mp3`);
  });
});

describe("parseSession", () => {
  const json = (over: Record<string, unknown> = {}) =>
    JSON.stringify({ ...snapshotSession(source())!, ...over });

  it("round-trips a snapshot", () => {
    expect(parseSession(json())).toEqual(snapshotSession(source()));
  });

  it("rejects missing, malformed or foreign data", () => {
    expect(parseSession(null)).toBeNull();
    expect(parseSession("")).toBeNull();
    expect(parseSession("{not json")).toBeNull();
    expect(parseSession(JSON.stringify({ v: 2, queue: [], order: [] }))).toBeNull();
    expect(parseSession(JSON.stringify([1, 2]))).toBeNull();
  });

  it("drops invalid tracks and re-points the order at what is left", () => {
    const queue = [local(0), { id: "bad", source: "local", title: "no payload" }, local(2), local(3)];
    const parsed = parseSession(json({ queue, order: [3, 1, 2, 0], orderPos: 2 }))!;
    expect(parsed.queue.map((q) => q.id)).toEqual(["/m/0.mp3", "/m/2.mp3", "/m/3.mp3"]);
    // Old 3,2,0 → new 2,1,0; the current track (old index 2) is still current.
    expect(parsed.order).toEqual([2, 1, 0]);
    expect(parsed.queue[parsed.order[parsed.orderPos]!]?.id).toBe("/m/2.mp3");
  });

  it("appends tracks a stale order forgot and drops duplicates", () => {
    const parsed = parseSession(json({ order: [1, 1, 0], orderPos: 0 }))!;
    expect(parsed.order).toEqual([1, 0, 2]);
  });

  it("has nothing to resume when the current track itself is unusable", () => {
    const queue = [local(0), { id: "bad", source: "ytmusic", title: "x" }, local(2)];
    expect(parseSession(json({ queue, order: [0, 1, 2], orderPos: 1 }))).toBeNull();
    expect(parseSession(json({ orderPos: 7 }))).toBeNull();
  });

  it("never resurrects a cast, and needs the phone's device for a phone track", () => {
    const cast = { id: "c", source: "cast", title: "Cast" };
    expect(parseSession(json({ queue: [cast], order: [0], orderPos: 0 }))).toBeNull();
    const phone = { id: "p", source: "phone", title: "P", phoneTrack: { id: "1" } };
    expect(parseSession(json({ queue: [phone], order: [0], orderPos: 0 }))).toBeNull();
    const ok = { ...phone, device: { id: "dev" } };
    expect(parseSession(json({ queue: [ok], order: [0], orderPos: 0 }))?.queue).toHaveLength(1);
  });

  it("defaults bad shuffle/repeat/position values", () => {
    const parsed = parseSession(json({ shuffle: "yes", repeat: "sometimes", positionSecs: -5 }))!;
    expect(parsed).toMatchObject({ shuffle: false, repeat: "off", positionSecs: 0 });
  });
});
