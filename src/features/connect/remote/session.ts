/**
 * The controller half of Hype Connect on a player (contract §3.12): what the
 * other players are doing, whether this app is in *remote mode*, and how to
 * command the device that is playing.
 *
 * Pure of I/O — frames go out through an injected `send`, time comes from
 * injected clocks — so the rules are tested directly.
 *
 * - Every other player's latest {@link StampedState} is kept (older `seq`
 *   dropped, forgotten when the device goes offline or on `welcome`).
 * - Remote mode = connected and `activeDeviceId` names another *online*
 *   device with cap `player`; {@link RemoteSession.view} is that device's
 *   state, with a small optimistic overlay so a tap feels instant.
 * - {@link RemoteSession.cmd} sends a `cmd` and resolves with the matching
 *   `ack` (or a client-side timeout a little after the hub's own 10 s).
 */
import type {
  AckError,
  Command,
  DeviceInfo,
  Frame,
  PlayerState,
  QueueEntry,
  RepeatMode,
  TrackRef,
} from "../protocol";
import { isOnlinePlayer } from "./takeover";

/** The hub answers `timeout` after 10 s; give that reply time to arrive. */
export const CMD_TIMEOUT_MS = 11_000;
/** How long a tapped control shows its optimistic value without a newer state. */
export const OVERLAY_MS = 3_000;

export type CmdResult = { ok: true } | { ok: false; error: AckError };

/** A player's state as controllers see it: stamped with the hub's clock. */
export type StampedState = PlayerState & { at: number };

/** The remote device as the player UI shows it. The position is a base value:
 *  {@link positionNow} adds the time elapsed since `at`. */
export interface RemoteView {
  device: DeviceInfo;
  seq: number;
  track: TrackRef | null;
  playing: boolean;
  /** Position at hub time `at`. */
  positionMs: number;
  at: number;
  durationMs: number | null;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
  queue: QueueEntry[];
}

/** `positionMs + (playing ? hubNow - at : 0)`, clamped to the duration (§3.6). */
export function positionNow(v: Pick<RemoteView, "positionMs" | "at" | "playing" | "durationMs">, hubNowMs: number): number {
  const raw = v.positionMs + (v.playing ? hubNowMs - v.at : 0);
  const floored = Math.max(0, raw);
  return v.durationMs !== null ? Math.min(floored, Math.max(0, v.durationMs)) : floored;
}

export interface SessionDeps {
  /** The hub's epoch ms now (the hub clock, §3.7). */
  hubNow(): number;
  send(frame: Frame): Promise<void>;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  /** The view changed (state, devices, overlay). `null` = not in remote mode. */
  onChange?(view: RemoteView | null): void;
  /** The device we were remote-controlling is gone (offline or unlinked). */
  onTargetLost?(device: DeviceInfo): void;
}

interface Overlay {
  baseSeq: number;
  playing?: boolean;
  position?: { ms: number; at: number };
  volume?: number;
  shuffle?: boolean;
  repeat?: RepeatMode;
}

interface Pending {
  resolve(r: CmdResult): void;
  timer: unknown;
}

export class RemoteSession {
  private selfId = "";
  private connected = false;
  private devices: readonly DeviceInfo[] = [];
  private activeId: string | null = null;
  private states = new Map<string, StampedState>();
  private overlay: { deviceId: string; o: Overlay } | null = null;
  private overlayTimer: unknown = null;
  private pending = new Map<string, Pending>();
  private counter = 0;
  private lastTarget: DeviceInfo | null = null;
  private lastView: RemoteView | null = null;

  constructor(private readonly deps: SessionDeps) {}

  setSelfId(id: string): void {
    this.selfId = id;
    this.states.delete(id);
    this.emit();
  }

  setConnected(connected: boolean): void {
    if (this.connected === connected) return;
    this.connected = connected;
    if (!connected) {
      this.states.clear();
      this.dropOverlay();
      this.failPending("offline");
    }
    this.emit();
  }

  /** A `welcome`: devices, active and every online player's latest state. */
  onWelcome(devices: readonly DeviceInfo[], activeId: string | null, states: Record<string, PlayerState>): void {
    this.states.clear();
    this.dropOverlay();
    this.devices = devices;
    this.activeId = activeId;
    const hubNow = this.deps.hubNow();
    for (const [id, s] of Object.entries(states)) this.store(id, s, hubNow);
    this.afterDevices();
  }

  onDevices(devices: readonly DeviceInfo[], activeId: string | null): void {
    this.devices = devices;
    this.activeId = activeId;
    for (const id of [...this.states.keys()]) {
      const d = devices.find((x) => x.id === id);
      if (!d || !d.online) this.states.delete(id);
    }
    this.afterDevices();
  }

  onState(deviceId: string | undefined, state: PlayerState): void {
    if (deviceId === undefined) return;
    if (!this.store(deviceId, state, this.deps.hubNow())) return;
    if (this.overlay?.deviceId === deviceId && state.seq > this.overlay.o.baseSeq) this.dropOverlay();
    this.emit();
  }

  onAck(id: string, ok: boolean, error?: AckError): void {
    const p = this.pending.get(id);
    if (!p) return;
    this.pending.delete(id);
    this.deps.clearTimeout(p.timer);
    p.resolve(ok ? { ok: true } : { ok: false, error: error ?? "failed" });
  }

  /** The device the app is remote-controlling, or null (local mode). */
  get target(): DeviceInfo | null {
    if (!this.connected || this.activeId === null || this.activeId === this.selfId) return null;
    const d = this.devices.find((x) => x.id === this.activeId);
    return isOnlinePlayer(d) ? d : null;
  }

  /** Every online player, this device included, for the picker. */
  get players(): DeviceInfo[] {
    return this.connected ? this.devices.filter(isOnlinePlayer) : [];
  }

  get active(): string | null {
    return this.activeId;
  }

  /** The latest known state of any other player (also the non-active ones). */
  stateOf(deviceId: string): StampedState | null {
    return this.states.get(deviceId) ?? null;
  }

  /** The target's state with the optimistic overlay, or null in local mode. */
  view(): RemoteView | null {
    const device = this.target;
    if (!device) return null;
    const s = this.states.get(device.id);
    const base: RemoteView = s
      ? {
          device,
          seq: s.seq,
          track: s.track,
          playing: s.playing,
          positionMs: s.positionMs,
          at: s.at,
          durationMs: s.durationMs,
          volume: s.volume,
          shuffle: s.shuffle,
          repeat: s.repeat,
          queue: s.queue,
        }
      : {
          device,
          seq: -1,
          track: null,
          playing: false,
          positionMs: 0,
          at: this.deps.hubNow(),
          durationMs: null,
          volume: 1,
          shuffle: false,
          repeat: "off",
          queue: [],
        };
    const o = this.overlay;
    if (!o || o.deviceId !== device.id) return base;
    const x = o.o;
    return {
      ...base,
      playing: x.playing ?? base.playing,
      positionMs: x.position?.ms ?? base.positionMs,
      at: x.position?.at ?? base.at,
      volume: x.volume ?? base.volume,
      shuffle: x.shuffle ?? base.shuffle,
      repeat: x.repeat ?? base.repeat,
    };
  }

  /** Send `command` to `to`; resolves with the ack. Never rejects. */
  cmd(to: string, command: Command): Promise<CmdResult> {
    const id = `rc-${(this.counter += 1)}`;
    return new Promise<CmdResult>((resolve) => {
      const timer = this.deps.setTimeout(() => {
        if (this.pending.delete(id)) {
          this.dropOverlayFor(command);
          resolve({ ok: false, error: "timeout" });
        }
      }, CMD_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: (r) => {
          if (!r.ok) this.dropOverlayFor(command);
          resolve(r);
        },
        timer,
      });
      this.applyOverlay(to, command);
      this.deps.send({ t: "cmd", id, to, cmd: command }).catch(() => {
        const p = this.pending.get(id);
        if (!p) return;
        this.pending.delete(id);
        this.deps.clearTimeout(p.timer);
        p.resolve({ ok: false, error: "failed" });
      });
    });
  }

  /** Tell the hub who should play (`select`). */
  select(deviceId: string | null): Promise<void> {
    return this.deps.send({ t: "select", deviceId });
  }

  dispose(): void {
    this.dropOverlay();
    this.failPending("offline");
  }

  /* ----------------------------------------------------------- internals */

  private store(deviceId: string, state: PlayerState, hubNow: number): boolean {
    if (deviceId === this.selfId) return false;
    const prev = this.states.get(deviceId);
    if (prev && state.seq <= prev.seq) return false;
    this.states.set(deviceId, { ...state, at: state.at ?? hubNow });
    return true;
  }

  private afterDevices(): void {
    const target = this.target;
    const prev = this.lastTarget;
    if (prev && target?.id !== prev.id) {
      const now = this.devices.find((d) => d.id === prev.id);
      if (!now || !now.online) this.deps.onTargetLost?.(prev);
    }
    this.lastTarget = target;
    if (this.overlay && this.overlay.deviceId !== target?.id) this.dropOverlay();
    this.emit();
  }

  private applyOverlay(to: string, command: Command): void {
    const s = this.states.get(to);
    const baseSeq = s?.seq ?? -1;
    const cur = this.overlay && this.overlay.deviceId === to ? this.overlay.o : null;
    const next: Overlay = { ...(cur ?? { baseSeq }), baseSeq: cur?.baseSeq ?? baseSeq };
    let touched = true;
    switch (command.type) {
      case "pause": {
        next.playing = false;
        next.position = this.currentPosition(to);
        break;
      }
      case "resume": {
        next.playing = true;
        next.position = this.currentPosition(to);
        break;
      }
      case "seek":
        next.position = { ms: Math.max(0, command.positionMs), at: this.deps.hubNow() };
        break;
      case "volume":
        next.volume = command.value;
        break;
      case "shuffle":
        next.shuffle = command.on;
        break;
      case "repeat":
        next.repeat = command.mode;
        break;
      default:
        touched = false;
    }
    if (!touched) return;
    this.overlay = { deviceId: to, o: next };
    if (this.overlayTimer !== null) this.deps.clearTimeout(this.overlayTimer);
    this.overlayTimer = this.deps.setTimeout(() => {
      this.overlayTimer = null;
      this.overlay = null;
      this.emit();
    }, OVERLAY_MS);
    this.emit();
  }

  /** Where playback is right now, pinned so a pause freezes the bar there. */
  private currentPosition(deviceId: string): { ms: number; at: number } | undefined {
    const s = this.states.get(deviceId);
    if (!s) return undefined;
    const hubNow = this.deps.hubNow();
    const live = this.overlay?.deviceId === deviceId ? this.overlay.o : null;
    const playing = live?.playing ?? s.playing;
    const base = live?.position ?? { ms: s.positionMs, at: s.at };
    return { ms: positionNow({ positionMs: base.ms, at: base.at, playing, durationMs: s.durationMs }, hubNow), at: hubNow };
  }

  /** The command failed: forget the guess it made. */
  private dropOverlayFor(command: Command): void {
    if (["pause", "resume", "seek", "volume", "shuffle", "repeat"].includes(command.type)) this.dropOverlay(true);
  }

  private dropOverlay(emit = false): void {
    if (this.overlayTimer !== null) this.deps.clearTimeout(this.overlayTimer);
    this.overlayTimer = null;
    const had = this.overlay !== null;
    this.overlay = null;
    if (emit && had) this.emit();
  }

  private failPending(error: AckError): void {
    for (const [id, p] of this.pending) {
      this.deps.clearTimeout(p.timer);
      this.pending.delete(id);
      p.resolve({ ok: false, error });
    }
  }

  private emit(): void {
    const v = this.view();
    this.lastView = v;
    this.deps.onChange?.(v);
  }

  /** The last view handed to `onChange` (for tests and late subscribers). */
  get current(): RemoteView | null {
    return this.lastView;
  }
}
