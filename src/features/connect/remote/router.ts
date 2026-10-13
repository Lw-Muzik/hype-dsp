/**
 * "Pick here, play there" (contract §3.12): the ONE entry point every play
 * action in the app goes through.
 *
 * Not in remote mode → the caller's own local behaviour runs, untouched. In
 * remote mode the items become `TrackRef`s and go to the active device as
 * `play` / `enqueue`. What that device can't play (this computer's own files,
 * cloud and phone-link files, internet radio …) is not sent: if nothing is
 * left the user is asked whether to play it here instead — which, by Claim,
 * makes this computer the player — and if only some stayed behind they are
 * counted in a toast.
 */
import type { QueueItem } from "@/stores/engine";
import type { AckError, Command, DeviceInfo, EnqueueMode, TrackRef } from "../protocol";
import type { CmdResult } from "./session";

/** A `play` frame stays well under the hub's 256 KiB: ~300 B per ref. */
export const MAX_ITEMS_PER_PLAY = 400;
/** Items kept before the tapped one, so "previous" still has somewhere to go. */
const KEEP_BEFORE = 50;

export interface RouterDeps {
  /** The device the app is remote-controlling, or null for local mode. */
  target(): DeviceInfo | null;
  toRef(item: QueueItem): TrackRef;
  cmd(to: string, command: Command): Promise<CmdResult>;
  confirm(message: string): Promise<boolean>;
  info(message: string): void;
  error(message: string): void;
}

export interface PlayRequest {
  items: readonly QueueItem[];
  index: number;
  /** One tapped result whose local behaviour starts a radio. */
  radio?: boolean;
  /** What happens today: runs unchanged when not in remote mode, or when the
   *  user chooses to play here. */
  local: () => void;
}

export interface EnqueueRequest {
  items: readonly QueueItem[];
  mode: EnqueueMode;
  local: () => void;
}

const ACK_MESSAGES: Record<AckError, (device: string) => string> = {
  offline: (d) => `${d} is offline.`,
  not_a_player: (d) => `${d} can't play music.`,
  nothing_playable: (d) => `${d} can't play that.`,
  unsupported: (d) => `${d} doesn't support that yet. Update the app there.`,
  failed: (d) => `${d} couldn't do that.`,
  timeout: (d) => `${d} didn't answer.`,
};

export const ackMessage = (error: AckError, device: string): string => ACK_MESSAGES[error](device);

const isRemotePlayable = (ref: TrackRef): boolean => ref.src === "yt";

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** Which of `items` the target can play, with `index` re-based onto them:
 *  the requested item if it survived, else the next playable after it, else
 *  the closest one before it. */
export function selectPlayable(
  items: readonly QueueItem[],
  index: number,
  toRef: (i: QueueItem) => TrackRef,
): { refs: TrackRef[]; start: number; dropped: number } {
  const refs: TrackRef[] = [];
  let start = -1;
  let before = -1;
  items.forEach((item, i) => {
    const ref = toRef(item);
    if (!isRemotePlayable(ref)) return;
    if (i >= index && start < 0) start = refs.length;
    if (i < index) before = refs.length;
    refs.push(ref);
  });
  if (start < 0) start = Math.max(0, before);
  return { refs, start, dropped: items.length - refs.length };
}

/** At most {@link MAX_ITEMS_PER_PLAY} refs around `start`. */
function window(refs: TrackRef[], start: number): { refs: TrackRef[]; start: number } {
  if (refs.length <= MAX_ITEMS_PER_PLAY) return { refs, start };
  const from = Math.max(0, start - KEEP_BEFORE);
  return { refs: refs.slice(from, from + MAX_ITEMS_PER_PLAY), start: start - from };
}

export class Router {
  constructor(private readonly deps: RouterDeps) {}

  /** Whether plays currently go to another device. */
  get remote(): boolean {
    return this.deps.target() !== null;
  }

  async play(req: PlayRequest): Promise<void> {
    const target = this.deps.target();
    if (!target) {
      req.local();
      return;
    }
    const picked = selectPlayable(req.items, req.index, this.deps.toRef);
    if (picked.refs.length === 0) {
      const subject = req.items.length === 1 ? `"${req.items[0]!.title}"` : "These tracks";
      if (await this.deps.confirm(`${subject} ${req.items.length === 1 ? "is" : "are"} only on this computer. Play ${req.items.length === 1 ? "it" : "them"} here instead?`)) {
        req.local();
      }
      return;
    }
    if (picked.dropped > 0) {
      this.deps.info(`${plural(picked.dropped, "track stayed", "tracks stayed")} on this computer — ${target.name} can't play ${picked.dropped === 1 ? "it" : "them"}.`);
    }
    const { refs, start } = window(picked.refs, picked.start);
    const result = await this.deps.cmd(target.id, {
      type: "play",
      items: refs,
      index: start,
      ...(req.radio && refs.length === 1 ? { radio: true } : {}),
    });
    if (!result.ok) this.deps.error(ackMessage(result.error, target.name));
  }

  async enqueue(req: EnqueueRequest): Promise<void> {
    const target = this.deps.target();
    if (!target) {
      req.local();
      return;
    }
    const picked = selectPlayable(req.items, 0, this.deps.toRef);
    if (picked.refs.length === 0) {
      const one = req.items.length === 1;
      const subject = one ? `"${req.items[0]!.title}"` : "These tracks";
      if (await this.deps.confirm(`${subject} ${one ? "is" : "are"} only on this computer. Add ${one ? "it" : "them"} to this computer's queue instead?`)) {
        req.local();
      }
      return;
    }
    if (picked.dropped > 0) {
      this.deps.info(`${plural(picked.dropped, "track stayed", "tracks stayed")} on this computer — ${target.name} can't play ${picked.dropped === 1 ? "it" : "them"}.`);
    }
    const result = await this.deps.cmd(target.id, {
      type: "enqueue",
      items: picked.refs.slice(0, MAX_ITEMS_PER_PLAY),
      mode: req.mode,
    });
    if (!result.ok) this.deps.error(ackMessage(result.error, target.name));
  }
}

/* ------------------------------------------------------- the app's router */

let current: Router | null = null;

/** Mounted once by `useHypeConnect`; `null` turns routing off. */
export function configureRouter(router: Router | null): void {
  current = router;
}

/** Play `items` from `index`, here or on the active device. */
export function routePlay(req: PlayRequest): void {
  if (!current) req.local();
  else void current.play(req);
}

/** Queue `items`, here or on the active device. */
export function routeEnqueue(req: EnqueueRequest): void {
  if (!current) req.local();
  else void current.enqueue(req);
}

/** Whether a play right now would go to another device. */
export function isRemoteMode(): boolean {
  return current?.remote ?? false;
}
