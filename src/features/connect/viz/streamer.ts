/**
 * The visualiser stream (§3.11, v1.1): what this desktop's audio looks like,
 * sent to a browser that draws it, on the beat. The shape follows the phone's
 * `ConnectVizStreamer` (`hype/lib/services/connect/connect_viz.dart`).
 *
 * # Only while somebody is watching, and only while it plays
 *
 * The engine's frames (~30/s of IPC, plus the re-binning and the beat
 * detector here) are listened to exactly while all of these hold, and released
 * the moment one stops:
 *
 * - a controller subscribed (`viz_sub on:true` from the hub),
 * - the socket is online (a subscription belongs to a socket; the hub repeats
 *   it after the next `welcome`), and
 * - music is playing (a paused player sends nothing).
 *
 * # Frame rate
 *
 * A frame goes out every {@link VizOptions.intervalMs} (50 ms = 20 fps), never
 * closer than `minGapMs`, and never more than `maxPerSecond` in any sliding
 * second (the hub drops the excess silently, and the Rust client refuses it).
 * A bass onset that lands between two slots is sent at once (still honouring
 * the gap) instead of up to 50 ms late — the point is to be on the beat.
 *
 * # Timing
 *
 * `heardAt` is when the frame's audio reaches the listener, in the HUB's clock:
 * the hub time now plus the output latency. The engine's frame describes the
 * block it is writing to the output right now and exposes no device latency, so
 * the latency is 0 (a Bluetooth sink will look that much early). `positionMs`
 * is the playback position of that audio.
 */
import type { VizFrame } from "../protocol";
import { BassBeat } from "./bassBeat";
import { bassDb, clamp01, rebinBands, round2, vizLevel } from "./bands";

export interface VizOptions {
  /** Target spacing of frames. */
  intervalMs: number;
  /** Hard floor on spacing between two frames. */
  minGapMs: number;
  /** The hub drops frames beyond this many per second. */
  maxPerSecond: number;
}

export const DEFAULT_VIZ_OPTIONS: VizOptions = { intervalMs: 50, minGapMs: 25, maxPerSecond: 30 };

/** Frames closer than this are one stall's worth of jitter, further apart are a
 *  pause; the beat detector's clock is clamped to this range. */
const MIN_DT_S = 0.005;
const MAX_DT_S = 0.1;

export interface VizDeps {
  /** Start receiving engine spectra; returns the stop function. Only called
   *  while streaming is wanted. */
  listen(onSpectrum: (spectrum: readonly number[]) => void): () => void;
  /** Queue a frame for the hub; rejection means it didn't go. */
  send(frame: VizFrame): Promise<void>;
  isPlaying(): boolean;
  /** Playback position now, ms. */
  positionMs(): number;
  /** The hub's epoch ms now. */
  hubNowMs(): number;
  /** Monotonic ms. */
  nowMs(): number;
  /** Delay between the engine frame and the sound (0 when unknown). */
  outputLatencyMs?(): number;
}

export class VizStreamer {
  private subscribed = false;
  private online = false;
  private stop: (() => void) | null = null;

  private readonly bass = new BassBeat();
  private lastFrameAt: number | null = null;
  private lastSentAt = Number.NEGATIVE_INFINITY;
  private dueAt = 0;
  private pendingBeat = 0;
  private recentSends: number[] = [];

  private readonly options: VizOptions;

  constructor(
    private readonly deps: VizDeps,
    options: Partial<VizOptions> = {},
  ) {
    this.options = { ...DEFAULT_VIZ_OPTIONS, ...options };
  }

  /** Whether engine frames are being listened to. */
  get isStreaming(): boolean {
    return this.stop !== null;
  }

  /** A controller started (`true`) or stopped (`false`) watching. */
  setSubscribed(on: boolean): void {
    // A subscription belongs to a socket: one arriving with none up is stale,
    // and the hub repeats it after the next `welcome` anyway.
    this.subscribed = on && this.online;
    this.refresh();
  }

  /** The socket came up or went away. A subscription does not survive it. */
  setOnline(online: boolean): void {
    this.online = online;
    if (!online) this.subscribed = false;
    this.refresh();
  }

  /** Re-reads whether anything is playing; call when playback changes. */
  refresh(): void {
    const wanted = this.subscribed && this.online && this.deps.isPlaying();
    if (wanted && this.stop === null) {
      this.resetAnalysis();
      this.recentSends = [];
      this.lastSentAt = Number.NEGATIVE_INFINITY;
      this.dueAt = 0;
      this.stop = this.deps.listen((spectrum) => this.onSpectrum(spectrum));
    } else if (!wanted && this.stop !== null) {
      const stop = this.stop;
      this.stop = null;
      stop();
    }
  }

  /** A new track (or a seek) begins: the beat detector forgets the last song. */
  resetAnalysis(): void {
    this.bass.reset();
    this.pendingBeat = 0;
    this.lastFrameAt = null;
  }

  dispose(): void {
    this.subscribed = false;
    this.online = false;
    this.refresh();
  }

  private onSpectrum(spectrum: readonly number[]): void {
    const now = this.deps.nowMs();

    // Every frame feeds the detector, sent or not: it needs a continuous
    // signal, and a hit in a skipped frame is carried to the next one sent.
    const dt =
      this.lastFrameAt === null
        ? MAX_DT_S
        : Math.min(MAX_DT_S, Math.max(MIN_DT_S, (now - this.lastFrameAt) / 1000));
    this.lastFrameAt = now;
    const hit = this.bass.step(bassDb(spectrum), dt);
    if (hit && hit.strength > this.pendingBeat) this.pendingBeat = hit.strength;
    const onset = hit?.onset === true;

    const { intervalMs, minGapMs, maxPerSecond } = this.options;
    if (now - this.lastSentAt < minGapMs) return;
    this.recentSends = this.recentSends.filter((t) => now - t < 1000);
    if (this.recentSends.length >= maxPerSecond) return;
    const due = now >= this.dueAt;
    if (!due && !onset) return;
    if (!this.deps.isPlaying()) return;

    const latency = Math.max(0, this.deps.outputLatencyMs?.() ?? 0);
    const frame: VizFrame = {
      heardAt: Math.round(this.deps.hubNowMs() + latency),
      positionMs: Math.max(0, Math.round(this.deps.positionMs() + latency)),
      bands: rebinBands(spectrum),
      level: vizLevel(spectrum),
      beat: round2(clamp01(this.pendingBeat)),
    };

    this.deps.send(frame).catch(() => {});
    this.pendingBeat = 0;
    this.lastSentAt = now;
    this.recentSends.push(now);
    if (due) {
      // Keep the cadence rather than drifting by each arrival's jitter (so
      // ~31 engine frames a second still yield 20 a second); after a stall do
      // not try to catch up. An early onset leaves the schedule be.
      const next = this.dueAt + intervalMs;
      this.dueAt = next > now ? next : now + intervalMs;
    }
  }
}
