import type { QueueItem, QueueSource, RepeatMode } from "@/stores/engine";

/**
 * The playback session that survives a relaunch: the queue, where in it the
 * listener was, how far into the track, and shuffle/repeat. Same model as the
 * mobile app (`lib/services/playback_session.dart`): restoring rebuilds the
 * queue but loads nothing; the first press of play loads the track and seeks
 * to the saved position. A restored session is always paused: an app that
 * makes noise because it was opened is a bad guest.
 *
 * Pure: serialisation, windowing and validation only. Persistence lives in
 * `useSessionPersistence`, the restore/resume behaviour in the engine store.
 */

export const SESSION_VERSION = 1;
/** Longest queue kept, windowed around the current track. */
export const MAX_SESSION_TRACKS = 500;
/** How many already-played tracks the window keeps behind the current one. */
export const SESSION_TRACKS_BEHIND = 50;

export interface PlaybackSession {
  v: typeof SESSION_VERSION;
  queue: QueueItem[];
  /** Play order: indices into `queue` (a permutation when shuffled). */
  order: number[];
  /** Position of the current track within `order`. */
  orderPos: number;
  shuffle: boolean;
  repeat: RepeatMode;
  /** Seconds into the current track. */
  positionSecs: number;
}

/** The engine-store fields a session is taken from. */
export interface SessionSource {
  queue: QueueItem[];
  order: number[];
  orderPos: number;
  shuffle: boolean;
  repeat: RepeatMode;
  positionSecs: number;
  /** Set while a restored session hasn't been played yet: its saved position. */
  resumeAt: number | null;
}

/** Sources a session can bring back. A phone cast is the phone's playback, not ours. */
const RESTORABLE: ReadonlySet<QueueSource> = new Set(["local", "phone", "cloud", "ytmusic", "radio"]);

/** The payload field each source needs to play again. */
const PAYLOAD: Record<Exclude<QueueSource, "cast">, keyof QueueItem> = {
  local: "track",
  phone: "phoneTrack",
  cloud: "cloud",
  ytmusic: "ytTrack",
  radio: "radioUrl",
};

/** A copy fit to store: inline (`data:`) cover art is dropped; it is re-read. */
function storable(item: QueueItem): QueueItem {
  if (typeof item.cover === "string" && item.cover.startsWith("data:")) {
    const { cover: _cover, ...rest } = item;
    return rest;
  }
  return item;
}

/**
 * The session to save for `s`, or null when there is nothing to come back to
 * (an empty queue, or a cast from the phone). Long queues are windowed to
 * {@link MAX_SESSION_TRACKS} around the current track, in play order.
 */
export function snapshotSession(s: SessionSource): PlaybackSession | null {
  const current = s.order[s.orderPos];
  const item = current === undefined ? undefined : s.queue[current];
  if (!item || !RESTORABLE.has(item.source)) return null;
  // A live station has no position; a track's is the saved one if it hasn't
  // been played since the restore.
  const positionSecs = item.source === "radio" ? 0 : Math.max(0, s.resumeAt ?? s.positionSecs);

  if (s.order.length <= MAX_SESSION_TRACKS) {
    return {
      v: SESSION_VERSION,
      queue: s.queue.map(storable),
      order: [...s.order],
      orderPos: s.orderPos,
      shuffle: s.shuffle,
      repeat: s.repeat,
      positionSecs,
    };
  }

  // Window the play order, not the unshuffled list: the latter would drop
  // tracks the shuffled order still points at. The window becomes the queue,
  // in play order.
  const start = Math.max(
    0,
    Math.min(s.orderPos - SESSION_TRACKS_BEHIND, s.order.length - MAX_SESSION_TRACKS),
  );
  const window = s.order.slice(start, start + MAX_SESSION_TRACKS);
  const queue = window
    .map((i) => s.queue[i])
    .filter((it): it is QueueItem => it !== undefined)
    .map(storable);
  return {
    v: SESSION_VERSION,
    queue,
    order: queue.map((_, i) => i),
    orderPos: s.orderPos - start,
    shuffle: s.shuffle,
    repeat: s.repeat,
    positionSecs,
  };
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

function validItem(v: unknown): v is QueueItem {
  if (!isObj(v)) return false;
  if (typeof v.id !== "string" || typeof v.title !== "string") return false;
  const source = v.source as QueueSource;
  if (!RESTORABLE.has(source)) return false;
  const payload = v[PAYLOAD[source as Exclude<QueueSource, "cast">]];
  if (source === "radio") return typeof payload === "string" && payload.length > 0;
  if (!isObj(payload)) return false;
  // Phone tracks also need their device to stream from.
  if (source === "phone" && !isObj(v.device)) return false;
  return true;
}

/**
 * Parse and repair a saved session; null if it can't be used. Invalid items
 * are dropped and the order re-pointed; entries the order lost are appended
 * so a queue can never lose tracks to a stale index. If the current track
 * itself is unusable there is nothing sensible to resume.
 */
export function parseSession(text: string | null | undefined): PlaybackSession | null {
  if (!text) return null;
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObj(raw) || raw.v !== SESSION_VERSION || !Array.isArray(raw.queue) || !Array.isArray(raw.order)) {
    return null;
  }

  // Keep valid items; map old indices to new ones.
  const queue: QueueItem[] = [];
  const remap = new Map<number, number>();
  raw.queue.forEach((it, i) => {
    if (validItem(it)) {
      remap.set(i, queue.length);
      queue.push(it);
    }
  });
  if (queue.length === 0) return null;

  const rawOrder = raw.order as unknown[];
  const rawPos = typeof raw.orderPos === "number" ? raw.orderPos : -1;
  const currentOld = Number.isInteger(rawOrder[rawPos]) ? (rawOrder[rawPos] as number) : undefined;
  const currentNew = currentOld === undefined ? undefined : remap.get(currentOld);
  if (currentNew === undefined) return null;

  // Re-point the order: valid, de-duplicated, then anything it forgot.
  const seen = new Set<number>();
  const order: number[] = [];
  for (const o of rawOrder) {
    const n = Number.isInteger(o) ? remap.get(o as number) : undefined;
    if (n !== undefined && !seen.has(n)) {
      seen.add(n);
      order.push(n);
    }
  }
  for (let i = 0; i < queue.length; i++) if (!seen.has(i)) order.push(i);

  const repeat: RepeatMode = raw.repeat === "all" || raw.repeat === "one" ? raw.repeat : "off";
  const positionSecs =
    typeof raw.positionSecs === "number" && Number.isFinite(raw.positionSecs) && raw.positionSecs > 0
      ? raw.positionSecs
      : 0;

  return {
    v: SESSION_VERSION,
    queue,
    order,
    orderPos: order.indexOf(currentNew),
    shuffle: raw.shuffle === true,
    repeat,
    positionSecs,
  };
}
