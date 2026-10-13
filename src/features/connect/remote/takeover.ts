/**
 * Claim and Yield (contract §3.12): one device plays at a time.
 *
 * A pure state machine — time, timers and the two side effects are injected —
 * so the rules (including the two-players race) are tested directly.
 *
 * - **Claim.** A connected player whose `activeDeviceId` isn't itself sends
 *   `select {self}` when (a) its playback goes from not playing to playing, or
 *   (b) a `welcome`/`devices` shows it playing while the active device is
 *   null, offline or not a player. It is then *claiming* until a
 *   `welcome`/`devices` names it active, the hub answers `not_a_player`, or
 *   3 s pass. At most one claim per 3 s.
 * - **Yield.** A player that is playing, not claiming, and sees a
 *   `welcome`/`devices` naming *another online player* active pauses (pause,
 *   not stop). A claiming player postpones the check until claiming ends and
 *   applies it to the latest list.
 *
 * - **Claim (c), on `welcome` only.** Claim rather than yield when playback
 *   started while there was no socket, or the active device's `welcome.states`
 *   entry is missing or not playing (a stale selection, e.g. after a merge).
 *   Yield on `welcome` only when neither applies; `devices` frames are
 *   unchanged, and a postponed check after a claim runs as a `devices` check.
 *
 * "Playing" is the music engine only; the Movies/TV player never reaches this.
 */
import type { DeviceInfo, PlayerState } from "../protocol";

export const CLAIM_TIMEOUT_MS = 3_000;
export const CLAIM_MIN_GAP_MS = 3_000;

export interface TakeoverDeps {
  /** Epoch ms (any monotonic-enough clock). */
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  /** Send `select {deviceId: selfId}`. */
  sendSelect(selfId: string): void;
  /** Pause local playback (queue and position stay). */
  pauseLocal(): void;
}

export const isOnlinePlayer = (d: DeviceInfo | undefined): d is DeviceInfo =>
  d !== undefined && d.online && d.caps.includes("player");

export class Takeover {
  private selfId = "";
  private connected = false;
  private devices: readonly DeviceInfo[] = [];
  private activeId: string | null = null;
  private playing = false;
  private claiming = false;
  private claimTimer: unknown = null;
  private lastClaimAt: number | null = null;
  /** Playback started while there was no socket (set on that play edge,
   *  cleared when playback stops or a claim is sent). */
  private offlineStart = false;

  constructor(private readonly deps: TakeoverDeps) {}

  setSelfId(id: string): void {
    this.selfId = id;
  }

  /** The hub connection came up (`true`, after `welcome`) or went away. */
  setConnected(connected: boolean): void {
    if (this.connected === connected) return;
    this.connected = connected;
    if (!connected) this.endClaim();
  }

  /** Local music playback changed (`playing && !paused`). Only an edge acts. */
  setPlaying(playing: boolean): void {
    const was = this.playing;
    this.playing = playing;
    if (!playing) this.offlineStart = false;
    if (playing && !was) {
      if (!this.connected) this.offlineStart = true;
      this.claim();
    }
  }

  /** A `welcome` frame (§3.12 Claim (c) / Yield differ from `devices`). */
  onWelcome(
    devices: readonly DeviceInfo[],
    activeDeviceId: string | null,
    states: Readonly<Record<string, PlayerState>>,
  ): void {
    this.devices = devices;
    this.activeId = activeDeviceId;
    if (activeDeviceId === this.selfId && this.selfId !== "") {
      this.endClaim();
      return;
    }
    if (this.claiming || !this.connected || !this.playing) return;
    const active = activeDeviceId === null ? undefined : devices.find((d) => d.id === activeDeviceId);
    if (isOnlinePlayer(active)) {
      const activePlaying = states[active.id]?.playing === true;
      if (this.offlineStart || !activePlaying) this.claim();
      else this.deps.pauseLocal();
    } else {
      this.claim();
    }
  }

  /** A `devices` frame. */
  onDevices(devices: readonly DeviceInfo[], activeDeviceId: string | null): void {
    this.devices = devices;
    this.activeId = activeDeviceId;
    if (activeDeviceId === this.selfId && this.selfId !== "") {
      this.endClaim();
      return;
    }
    if (this.claiming) return; // postponed until the claim ends
    this.evaluate();
  }

  /** `error {code}` from the hub. */
  onError(code: string): void {
    if (code === "not_a_player" && this.claiming) this.endClaim();
  }

  dispose(): void {
    this.clearTimer();
  }

  get isClaiming(): boolean {
    return this.claiming;
  }

  /** (b) claim, or the yield check, against the latest device list. */
  private evaluate(): void {
    if (!this.connected || !this.playing) return;
    const active = this.activeId === null ? undefined : this.devices.find((d) => d.id === this.activeId);
    if (this.activeId !== null && this.activeId !== this.selfId && isOnlinePlayer(active)) {
      this.deps.pauseLocal();
    } else if (this.activeId !== this.selfId) {
      this.claim(); // null, offline or not a player
    }
  }

  private claim(): void {
    if (!this.connected || this.selfId === "" || this.activeId === this.selfId) return;
    const now = this.deps.now();
    if (this.lastClaimAt !== null && now - this.lastClaimAt < CLAIM_MIN_GAP_MS) return;
    this.lastClaimAt = now;
    this.offlineStart = false;
    this.claiming = true;
    this.clearTimer();
    this.claimTimer = this.deps.setTimeout(() => {
      this.claimTimer = null;
      this.endClaim();
    }, CLAIM_TIMEOUT_MS);
    this.deps.sendSelect(this.selfId);
  }

  /** Claiming is over; apply the postponed yield check (never a new claim). */
  private endClaim(): void {
    const was = this.claiming;
    this.claiming = false;
    this.clearTimer();
    if (!was || !this.connected || !this.playing) return;
    const active = this.activeId === null ? undefined : this.devices.find((d) => d.id === this.activeId);
    if (this.activeId !== this.selfId && isOnlinePlayer(active)) this.deps.pauseLocal();
  }

  private clearTimer(): void {
    if (this.claimTimer !== null) this.deps.clearTimeout(this.claimTimer);
    this.claimTimer = null;
  }
}
