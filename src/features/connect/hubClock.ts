/**
 * This desktop's view of the hub's clock (§3.7), done in TypeScript: the
 * `ping`/`pong` frames already cross the IPC boundary, and everything that
 * needs hub time (a viz frame's `heardAt`) lives here too.
 *
 * # One sample
 *
 * `ping` leaves at local time `sent`; `pong` returns at `received` carrying the
 * hub's `serverTime`. The hub answered somewhere in between; assuming the
 * middle, the hub clock read `serverTime` when ours read `(sent + received) / 2`:
 *
 *     offset = serverTime - (sent + received) / 2
 *
 * That assumption errs by at most half the round trip, so the best sample is
 * the one with the *smallest* round trip. {@link HubClock} keeps the last
 * {@link CLOCK_WINDOW} samples and trusts the fastest: one slow exchange can't
 * move the estimate, and a good old one ages out after five samples instead of
 * pinning the offset forever (clocks drift; users change them).
 *
 * {@link ClockSync} takes a sample on every `welcome` and every
 * {@link CLOCK_INTERVAL_MS} after.
 */

/** Samples compared. */
export const CLOCK_WINDOW = 5;
/** Re-measure this often while connected. */
export const CLOCK_INTERVAL_MS = 10_000;
/** A round trip longer than this measures nothing (a clock set mid-flight, a
 *  pong that sat in a queue for ages). */
export const MAX_ROUND_TRIP_MS = 10_000;

interface Sample {
  offsetMs: number;
  roundTripMs: number;
}

export class HubClock {
  private samples: Sample[] = [];
  private seeded: number | null = null;

  constructor(
    private readonly localNow: () => number = () => Date.now(),
    private readonly window: number = CLOCK_WINDOW,
  ) {}

  /** Record one exchange. Returns `false` when it was unusable. */
  addSample(sentMs: number, receivedMs: number, serverTimeMs: number): boolean {
    const roundTripMs = receivedMs - sentMs;
    if (!Number.isFinite(roundTripMs) || !Number.isFinite(serverTimeMs)) return false;
    if (roundTripMs < 0 || roundTripMs > MAX_ROUND_TRIP_MS) return false;
    this.samples.push({
      offsetMs: Math.round(serverTimeMs - (sentMs + receivedMs) / 2),
      roundTripMs,
    });
    while (this.samples.length > this.window) this.samples.shift();
    return true;
  }

  /** A first guess from `welcome`, used only until a real sample arrives. The
   *  hello→welcome trip includes token verification so it measures nothing, but
   *  a clock off by that is better than one off by the whole offset. */
  seedFromWelcome(serverTimeMs: number, receivedMs: number): void {
    if (this.seeded === null && Number.isFinite(serverTimeMs)) {
      this.seeded = Math.round(serverTimeMs - receivedMs);
    }
  }

  /** Hub time minus local time; `null` before any sample or seed. */
  get offsetMs(): number | null {
    return this.best()?.offsetMs ?? this.seeded;
  }

  /** Whether a real ping/pong exchange has been measured (a seed isn't one). */
  get isMeasured(): boolean {
    return this.samples.length > 0;
  }

  /** Round trip of the sample the offset comes from. */
  get roundTripMs(): number | null {
    return this.best()?.roundTripMs ?? null;
  }

  /** The hub's epoch ms now. Before a measurement this is the local clock —
   *  off by the offset, but never absent. */
  nowMs(): number {
    return this.localNow() + (this.offsetMs ?? 0);
  }

  /** Forget everything (a new account / socket). */
  clear(): void {
    this.samples = [];
    this.seeded = null;
  }

  /** The fastest kept sample; on a tie the newest. */
  private best(): Sample | null {
    let best: Sample | null = null;
    for (const s of this.samples) {
      if (best === null || s.roundTripMs <= best.roundTripMs) best = s;
    }
    return best;
  }
}

export interface ClockSyncDeps {
  clock: HubClock;
  sendPing(id: string): Promise<void>;
  /** Local epoch ms. */
  now(): number;
  setInterval(fn: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
}

/** Pings the hub on connect and then periodically; feeds the pongs to a
 *  {@link HubClock}. */
export class ClockSync {
  private pending = new Map<string, number>();
  private timer: unknown = null;
  private counter = 0;

  constructor(private readonly deps: ClockSyncDeps) {}

  /** The socket came up (a `welcome` arrived): measure now, then every 10 s. */
  start(): void {
    this.stop();
    this.ping();
    this.timer = this.deps.setInterval(() => this.ping(), CLOCK_INTERVAL_MS);
  }

  /** The socket is gone. The measured offset is kept — it describes the two
   *  clocks, not the connection — but nothing is in flight any more. */
  stop(): void {
    if (this.timer !== null) this.deps.clearInterval(this.timer);
    this.timer = null;
    this.pending.clear();
  }

  /** A `pong` arrived. */
  onPong(id: string, serverTime: number): void {
    const sent = this.pending.get(id);
    if (sent === undefined) return;
    this.pending.delete(id);
    this.deps.clock.addSample(sent, this.deps.now(), serverTime);
  }

  private ping(): void {
    const id = `clk-${(this.counter += 1)}`;
    // Only the freshest few can still be answered usefully.
    if (this.pending.size >= CLOCK_WINDOW) {
      const oldest = this.pending.keys().next().value;
      if (oldest !== undefined) this.pending.delete(oldest);
    }
    this.pending.set(id, this.deps.now());
    this.deps.sendPing(id).catch(() => this.pending.delete(id));
  }
}
