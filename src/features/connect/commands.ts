/**
 * Remote commands → the app's own player.
 *
 * Every command is mapped onto the same engine-store actions the UI uses, so
 * a remote `play` rides exactly the streaming/gapless/radio stack a click
 * does. Two rules from the contract shape everything here:
 *
 * - **Explicit states, never toggles.** `pause` while paused, `shuffle on`
 *   while on: no-ops, never flips. Two controllers racing can't invert state.
 * - **Never navigate.** Nothing here opens a view or steals focus; the app's
 *   UI simply follows the store like it does for the OS media keys.
 *
 * The ack goes out once the command is *accepted* (queued/applied), not when
 * audio starts; the next `state` shows the outcome.
 */
import { localItem, ytmusicItem, type QueueItem } from "@/stores/engine";
import type { LibraryTrack, YtTrack } from "@/lib/types";
import type { AckError, Command, RepeatMode, TrackRef } from "./protocol";
import type { QueueUidTracker } from "./queueUids";
import { msToSecs, ytRefToTrack } from "./refs";

/** What the executor needs to see of the player. */
export interface PlayerSnapshot {
  playing: boolean;
  paused: boolean;
  seekable: boolean;
  queue: readonly QueueItem[];
  order: readonly number[];
  orderPos: number;
}

/** The player, as far as remote commands are concerned. The real one is
 *  `enginePlayerPort()` over the engine store; tests use a fake. */
export interface PlayerPort {
  /** This desktop's Connect device id (its install id). */
  deviceId(): string;
  /** A track from this device's library by path, or null if it isn't there. */
  findLocal(path: string): Promise<LibraryTrack | null>;
  snapshot(): PlayerSnapshot;
  playItems(items: QueueItem[], index: number): void;
  /** Play `seed` and let YT Music's radio grow the queue behind it — what
   *  tapping a search result in the app does. */
  playRadio(seed: YtTrack): void;
  /** Seek to `secs` once the track that was just started can seek. */
  startAt(secs: number): void;
  enqueue(items: QueueItem[], mode: "next" | "end"): void;
  pause(): void;
  resume(): void;
  jumpTo(orderPos: number): void;
  next(): void;
  prev(): void;
  seek(secs: number): void;
  stop(): Promise<void>;
  setVolume(value: number): void;
  setShuffle(on: boolean): void;
  setRepeat(mode: RepeatMode): void;
}

export type AckOutcome = { ok: true } | { ok: false; error: AckError };

const OK: AckOutcome = { ok: true };
const refuse = (error: AckError): AckOutcome => ({ ok: false, error });

/** A ref as a queue item this desktop can play, or null to skip it: another
 *  device's `local` track, a path that isn't in the library, or an `other`
 *  ref (which belongs in `state` only). */
async function playable(ref: TrackRef, port: PlayerPort): Promise<QueueItem | null> {
  switch (ref.src) {
    case "yt":
      return ytmusicItem(ytRefToTrack(ref));
    case "local": {
      if (ref.deviceId !== port.deviceId()) return null;
      const track = await port.findLocal(ref.id);
      return track ? localItem(track) : null;
    }
    case "other":
      return null;
  }
}

/** Resolve `refs`, dropping what can't play here, and re-base `index` onto
 *  what's left: the requested item if it survived, else the next playable
 *  one after it, else the closest one before it. */
export async function resolvePlayable(
  refs: readonly TrackRef[],
  index: number,
  port: PlayerPort,
): Promise<{ items: QueueItem[]; start: number }> {
  const resolved = await Promise.all(refs.map((r) => playable(r, port)));
  const items: QueueItem[] = [];
  let start = -1;
  let before = -1;
  resolved.forEach((item, i) => {
    if (!item) return;
    if (i >= index && start < 0) start = items.length;
    if (i < index) before = items.length;
    items.push(item);
  });
  if (start < 0) start = Math.max(0, before);
  return { items, start };
}

/** Execute one command. Never throws: a failure is an ack. */
export async function executeCommand(
  cmd: Command,
  port: PlayerPort,
  uids: QueueUidTracker,
): Promise<AckOutcome> {
  try {
    return await execute(cmd, port, uids);
  } catch {
    return refuse("failed");
  }
}

async function execute(cmd: Command, port: PlayerPort, uids: QueueUidTracker): Promise<AckOutcome> {
  switch (cmd.type) {
    case "play": {
      const { items, start } = await resolvePlayable(cmd.items, cmd.index, port);
      const first = items[start];
      if (!first) return refuse("nothing_playable");
      if (cmd.radio && first.ytTrack) {
        // One tapped result: it plays, its radio fills in behind it.
        port.playRadio(first.ytTrack);
      } else {
        port.playItems(items, start);
      }
      const at = msToSecs(cmd.positionMs);
      if (at !== null && at > 0) port.startAt(at);
      return OK;
    }
    case "enqueue": {
      const { items } = await resolvePlayable(cmd.items, 0, port);
      if (items.length === 0) return refuse("nothing_playable");
      port.enqueue(items, cmd.mode);
      return OK;
    }
    case "pause": {
      const s = port.snapshot();
      if (s.playing && !s.paused) port.pause();
      return OK;
    }
    case "resume": {
      const s = port.snapshot();
      if (s.playing) {
        if (s.paused) port.resume();
        return OK;
      }
      // Ended or stopped: replay the track the queue is parked on, if any.
      if (s.orderPos >= 0 && s.orderPos < s.order.length) {
        port.jumpTo(s.orderPos);
        return OK;
      }
      return refuse("nothing_playable");
    }
    case "stop":
      await port.stop();
      return OK;
    case "next":
      port.next();
      return OK;
    case "prev":
      port.prev();
      return OK;
    case "seek": {
      if (!port.snapshot().seekable) return refuse("unsupported");
      port.seek(Math.max(0, cmd.positionMs) / 1000);
      return OK;
    }
    case "jump": {
      const s = port.snapshot();
      const qi = uids.uidsFor(s.queue).indexOf(cmd.uid);
      const pos = qi < 0 ? -1 : s.order.indexOf(qi);
      if (pos < 0) return refuse("failed");
      port.jumpTo(pos);
      return OK;
    }
    case "volume":
      port.setVolume(Math.min(1, Math.max(0, cmd.value)));
      return OK;
    case "shuffle":
      port.setShuffle(cmd.on);
      return OK;
    case "repeat":
      port.setRepeat(cmd.mode);
      return OK;
  }
}

/**
 * Runs commands one at a time, in arrival order. A `play` that waits on the
 * library must not be overtaken by the `pause` sent right after it.
 */
export class CommandQueue {
  private tail: Promise<unknown> = Promise.resolve();

  run(task: () => Promise<void>): Promise<void> {
    const next = this.tail.then(task, task);
    this.tail = next.catch(() => {});
    return next;
  }
}
