/**
 * The player's half of §3.6: turn the engine store into `PlayerState` and
 * send it when something a controller shows changed — track, play/pause, a
 * seek, volume/shuffle/repeat, the queue — coalesced over 300 ms, plus a
 * heartbeat every 15 s while playing so controllers' extrapolated progress
 * never drifts far.
 */
import type { QueueItem, RepeatMode } from "@/stores/engine";
import type { PlayerState } from "./protocol";
import type { QueueUidTracker } from "./queueUids";
import { queueItemToRef, secsToMs } from "./refs";

/** Max upcoming entries in one state (§3.6). */
export const MAX_STATE_QUEUE = 50;
export const DEBOUNCE_MS = 300;
export const HEARTBEAT_MS = 15_000;
/** A position this far from where playback should be counts as a seek. */
export const SEEK_TOLERANCE_MS = 2_000;

/** The slice of the engine store a state is built from. */
export interface EngineView {
  playing: boolean;
  paused: boolean;
  nowPlaying: string | null;
  positionSecs: number;
  durationSecs: number | null;
  queue: readonly QueueItem[];
  queueIndex: number;
  order: readonly number[];
  orderPos: number;
  shuffle: boolean;
  repeat: RepeatMode;
  state: { masterVolume: number };
}

export type StateBody = Omit<PlayerState, "seq" | "at">;

/** The queue item a controller is told is the current track, or `undefined`
 *  when idle. After a natural end the queue stays parked on its last item, but
 *  nothing is loaded: that's idle, not "paused on track N". */
export function activeQueueItem(
  s: Pick<EngineView, "queue" | "queueIndex" | "playing" | "nowPlaying">,
): QueueItem | undefined {
  const current = s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined;
  return current !== undefined && (s.playing || s.nowPlaying !== null) ? current : undefined;
}

export function buildPlayerState(
  s: EngineView,
  deviceId: string,
  uids: QueueUidTracker,
): StateBody {
  const ids = uids.uidsFor(s.queue);
  const current = activeQueueItem(s);
  const active = current !== undefined;
  const upcoming = active
    ? s.order
        .slice(s.orderPos + 1, s.orderPos + 1 + MAX_STATE_QUEUE)
        .flatMap((qi) => {
          const item = s.queue[qi];
          const uid = ids[qi];
          return item && uid ? [{ uid, track: queueItemToRef(item, deviceId) }] : [];
        })
    : [];
  return {
    track: active ? queueItemToRef(current, deviceId) : null,
    playing: active && s.playing && !s.paused,
    positionMs: active ? (secsToMs(s.positionSecs) ?? 0) : 0,
    durationMs: active ? (secsToMs(s.durationSecs) ?? secsToMs(current.durationSecs)) : null,
    volume: Math.min(1, Math.max(0, s.state.masterVolume)),
    shuffle: s.shuffle,
    repeat: s.repeat,
    queue: upcoming,
  };
}

export interface PublisherDeps {
  build(): StateBody;
  /** Call `onChange` whenever the engine store changes; returns unsubscribe. */
  subscribe(onChange: () => void): () => void;
  send(state: PlayerState): Promise<void>;
  now(): number;
  /** Where the last `seq` survives a restart. Without it `seq` starts from
   *  the clock alone. */
  seqStore?: SeqStore;
}

/** Persistence for the last `seq` sent. Implementations must not throw. */
export interface SeqStore {
  load(): number | null;
  save(seq: number): void;
}

/** First `seq` of a run: the epoch in SECONDS (monotonic across restarts and
 *  inside 32-bit range), or one past the last persisted value if that is
 *  higher (a clock that stepped back, or a restart within the same second). */
export function firstSeq(nowMs: number, persisted: number | null): number {
  const fromClock = Math.floor(nowMs / 1000);
  return persisted !== null && Number.isFinite(persisted) ? Math.max(fromClock, persisted + 1) : fromClock;
}

const SEQ_KEY = "hm.connect.seq";

/** `seq` persisted in localStorage; every access is guarded (private mode,
 *  blocked storage, tests without a DOM). */
export function localStorageSeqStore(): SeqStore {
  return {
    load() {
      try {
        const raw = globalThis.localStorage?.getItem(SEQ_KEY);
        const n = raw === null || raw === undefined ? NaN : Number(raw);
        return Number.isSafeInteger(n) && n >= 0 ? n : null;
      } catch {
        return null;
      }
    },
    save(seq) {
      try {
        globalThis.localStorage?.setItem(SEQ_KEY, String(seq));
      } catch {
        // Best effort: the clock alone still keeps seq climbing.
      }
    },
  };
}

export interface StatePublisher {
  /** Begin publishing (sends the current state at once). */
  start(): void;
  stop(): void;
  /** Send the current state now (e.g. right after a command was applied). */
  publishNow(): void;
}

/** Everything a controller renders except the ticking position. */
function significant(s: StateBody): string {
  return JSON.stringify([
    s.track,
    s.playing,
    s.durationMs,
    s.volume,
    s.shuffle,
    s.repeat,
    s.queue.map((q) => q.uid),
  ]);
}

export function createStatePublisher(deps: PublisherDeps): StatePublisher {
  // `seq` is the last one sent; the first send uses `firstSeq`.
  let seq = firstSeq(deps.now(), deps.seqStore?.load() ?? null) - 1;
  let running = false;
  let unsubscribe: (() => void) | null = null;
  let debounce: ReturnType<typeof setTimeout> | null = null;
  let heartbeat: ReturnType<typeof setTimeout> | null = null;
  let last: { key: string; positionMs: number; playing: boolean; at: number } | null = null;

  const clearTimers = () => {
    if (debounce !== null) clearTimeout(debounce);
    if (heartbeat !== null) clearTimeout(heartbeat);
    debounce = heartbeat = null;
  };

  const flush = () => {
    if (debounce !== null) clearTimeout(debounce);
    debounce = null;
    if (!running) return;
    const body = deps.build();
    seq += 1;
    deps.seqStore?.save(seq);
    last = { key: significant(body), positionMs: body.positionMs, playing: body.playing, at: deps.now() };
    void deps.send({ seq, ...body }).catch(() => {});
    if (heartbeat !== null) clearTimeout(heartbeat);
    heartbeat = body.playing ? setTimeout(flush, HEARTBEAT_MS) : null;
  };

  const seeked = (body: StateBody): boolean => {
    if (!last) return false;
    const expected = last.positionMs + (last.playing ? deps.now() - last.at : 0);
    return Math.abs(body.positionMs - expected) > SEEK_TOLERANCE_MS;
  };

  const onChange = () => {
    if (!running || debounce !== null) return; // already coalescing
    const body = deps.build();
    if (!last || significant(body) !== last.key || seeked(body)) {
      debounce = setTimeout(flush, DEBOUNCE_MS);
    }
  };

  return {
    start() {
      if (running) return;
      running = true;
      unsubscribe = deps.subscribe(onChange);
      flush();
    },
    stop() {
      running = false;
      unsubscribe?.();
      unsubscribe = null;
      clearTimers();
      last = null;
    },
    publishNow: flush,
  };
}
