/**
 * "Play on <device>" (contract §3.9, §3.12): move playback between this
 * computer and the account's other players.
 *
 * - **this computer → other** — the §3.9 transfer with this device as the
 *   source: `play` (own track + upcoming queue + position) to the target and,
 *   once it acks, pause here and `select` the target.
 * - **other → this computer** — "Play on this computer": the other device's
 *   track + queue + interpolated position go through the app's own `play`
 *   command handler as if the hub had sent them, then the old device is told
 *   to `pause` (it would yield by itself), and this computer is selected.
 * - **other → other** — §3.9 exactly like the web does it.
 *
 * Items the target can't play (another device's files, `other` sources) are
 * counted and reported, never silently lost.
 */
import type { Command, TrackRef } from "../protocol";
import type { StateBody } from "../statePublisher";
import { ackMessage } from "./router";
import { positionNow, type CmdResult, type RemoteView, type StampedState } from "./session";

type PlayCommand = Extract<Command, { type: "play" }>;

export interface TransferDeps {
  selfId(): string;
  /** Who is playing right now: the active device when it is another online
   *  player, else this computer. */
  sourceId(): string;
  /** The other device's state (any online player), or null. */
  stateOf(deviceId: string): StampedState | null;
  /** This computer's own player as a state body (track + upcoming queue). */
  localState(): StateBody;
  hubNow(): number;
  deviceName(id: string): string;
  cmd(to: string, command: Command): Promise<CmdResult>;
  select(deviceId: string): Promise<void>;
  /** Run a `play` on this computer through the app's command handler. */
  playHere(cmd: PlayCommand): Promise<{ ok: true } | { ok: false; error: string }>;
  pauseLocal(): void;
  info(message: string): void;
  error(message: string): void;
}

export type MoveResult =
  | { ok: true; moved: number; dropped: number }
  | { ok: false; reason: "same" | "nothing" | "failed" };

/** Whether `targetId` can play `ref`: YT Music anywhere, a device's own files
 *  only on that device, `other` (cloud, radio …) nowhere. */
export function playableOn(ref: TrackRef, targetId: string): boolean {
  return ref.src === "yt" || (ref.src === "local" && ref.deviceId === targetId);
}

interface Snapshot {
  track: TrackRef | null;
  queue: { track: TrackRef }[];
  positionMs: number;
  playing: boolean;
}

/** `play` for `targetId` from what the source is playing; null when nothing
 *  on it can move. */
export function buildTransfer(
  snap: Omit<Snapshot, "playing">,
  targetId: string,
): { cmd: PlayCommand; moved: number; dropped: number } | null {
  const all: TrackRef[] = [...(snap.track ? [snap.track] : []), ...snap.queue.map((q) => q.track)];
  const items = all.filter((r) => playableOn(r, targetId));
  if (items.length === 0) return null;
  const currentSurvived = snap.track !== null && items[0] === snap.track;
  return {
    cmd: {
      type: "play",
      items,
      index: 0,
      ...(currentSurvived && snap.positionMs > 0 ? { positionMs: Math.round(snap.positionMs) } : {}),
    },
    moved: items.length,
    dropped: all.length - items.length,
  };
}

const droppedNote = (n: number, target: string): string =>
  `${n} ${n === 1 ? "track" : "tracks"} couldn't move — ${target} can't play ${n === 1 ? "it" : "them"}.`;

export async function moveTo(targetId: string, d: TransferDeps): Promise<MoveResult> {
  const self = d.selfId();
  const source = d.sourceId();
  if (targetId === source) return { ok: false, reason: "same" };

  const snap = readSource(source, self, d);
  const built = snap ? buildTransfer(snap, targetId) : null;

  if (!built) {
    if (snap && snap.playing && (snap.track !== null || snap.queue.length > 0)) {
      // Something is playing but none of it can move.
      d.error(`Nothing playing on ${d.deviceName(source)} can play on ${d.deviceName(targetId)}.`);
      return { ok: false, reason: "nothing" };
    }
    if (snap && (snap.track !== null || snap.queue.length > 0)) {
      // Paused on music that can't move: there's nothing to carry, only the
      // choice of player, and refusing would leave no way to switch.
      await d.select(targetId).catch(() => {});
      d.info(`${d.deviceName(targetId)} is the player now. What was on ${d.deviceName(source)} stayed there.`);
      return { ok: true, moved: 0, dropped: 0 };
    }
    // Idle source: just hand over the role.
    await d.select(targetId).catch(() => {});
    return { ok: true, moved: 0, dropped: 0 };
  }

  if (targetId === self) {
    const played = await d.playHere(built.cmd);
    if (!played.ok) {
      d.error("Couldn't start that here.");
      return { ok: false, reason: "failed" };
    }
    // The old player would yield by itself; say so explicitly for older ones.
    void d.cmd(source, { type: "pause" });
  } else {
    const result = await d.cmd(targetId, built.cmd);
    if (!result.ok) {
      d.error(ackMessage(result.error, d.deviceName(targetId)));
      return { ok: false, reason: "failed" };
    }
    if (source === self) d.pauseLocal();
    else void d.cmd(source, { type: "pause" });
  }
  await d.select(targetId).catch(() => {});
  if (built.dropped > 0) d.info(droppedNote(built.dropped, d.deviceName(targetId)));
  return { ok: true, moved: built.moved, dropped: built.dropped };
}

function readSource(source: string, self: string, d: TransferDeps): Snapshot | null {
  if (source === self) {
    const s = d.localState();
    return { track: s.track, queue: s.queue, positionMs: s.positionMs, playing: s.playing };
  }
  const s = d.stateOf(source);
  if (!s) return null;
  const view: Pick<RemoteView, "positionMs" | "at" | "playing" | "durationMs"> = s;
  return { track: s.track, queue: s.queue, positionMs: positionNow(view, d.hubNow()), playing: s.playing };
}
