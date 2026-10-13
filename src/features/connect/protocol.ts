/**
 * Hype Connect protocol v1 — the TypeScript side of
 * `hypemuzik-management/backend/docs/hype-connect.md`.
 *
 * Rust (`crates/hm-connect`) owns the socket and already validates every
 * frame it forwards, but the webview still decodes what crosses the IPC
 * boundary instead of casting it: `decodeFrame` returns a canonical object
 * (only the contract's keys, nullable fields kept as `null`, optional ones
 * omitted) or `null`. Every frame fixture round-trips through it in
 * `protocol.test.ts`.
 */

export type DeviceKind = "web" | "mobile" | "desktop";
export type Cap = "player" | "yt" | "local";

export interface DeviceHello {
  id: string;
  kind: DeviceKind;
  name: string;
  platform: string;
  appVersion: string;
  caps: Cap[];
}

export interface DeviceInfo {
  id: string;
  kind: DeviceKind;
  name: string;
  platform: string | null;
  appVersion: string | null;
  caps: Cap[];
  online: boolean;
  lastSeenAt: string;
}

export interface YtRef {
  src: "yt";
  videoId: string;
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  artUrl: string | null;
}

/** A track in one device's own library; the desktop's `id` is the file path. */
export interface LocalRef {
  src: "local";
  deviceId: string;
  id: string;
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  artUrl: null;
}

/** Cloud, phone-link, radio … — only ever in `state`, never in commands. */
export interface OtherRef {
  src: "other";
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  artUrl: string | null;
}

export type TrackRef = YtRef | LocalRef | OtherRef;

export type RepeatMode = "off" | "all" | "one";
export type EnqueueMode = "next" | "end";

export type Command =
  | { type: "play"; items: TrackRef[]; index: number; positionMs?: number; radio?: boolean }
  | { type: "enqueue"; items: TrackRef[]; mode: EnqueueMode }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "stop" }
  | { type: "next" }
  | { type: "prev" }
  | { type: "seek"; positionMs: number }
  | { type: "jump"; uid: string }
  | { type: "volume"; value: number }
  | { type: "shuffle"; on: boolean }
  | { type: "repeat"; mode: RepeatMode };

export type AckError =
  | "offline"
  | "not_a_player"
  | "nothing_playable"
  | "unsupported"
  | "failed"
  | "timeout";

export type RpcError = "offline" | "unsupported" | "not_signed_in" | "failed" | "timeout";
export type RpcMethod = "search" | "suggest" | "open" | "artist" | "library";

export interface QueueEntry {
  uid: string;
  track: TrackRef;
}

export interface PlayerState {
  seq: number;
  track: TrackRef | null;
  playing: boolean;
  positionMs: number;
  durationMs: number | null;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
  queue: QueueEntry[];
  /** The hub's arrival stamp; absent on what a player sends. */
  at?: number;
}

export type ItemKind = "song" | "video" | "album" | "playlist" | "artist";

export interface Item {
  kind: ItemKind;
  id: string;
  title: string;
  subtitle: string | null;
  thumbnail: string | null;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  hasVideo: boolean;
}

export interface Shelf {
  title: string;
  items: Item[];
}

export type SearchFilter = "all" | "songs" | "videos" | "albums" | "artists" | "playlists";

export type Frame =
  | { t: "hello"; v: number; token: string; device: DeviceHello }
  | {
      t: "welcome";
      v: number;
      deviceId: string;
      serverTime: number;
      devices: DeviceInfo[];
      activeDeviceId: string | null;
      states: Record<string, PlayerState>;
    }
  | { t: "devices"; devices: DeviceInfo[]; activeDeviceId: string | null }
  | { t: "state"; deviceId?: string; state: PlayerState }
  | { t: "error"; code: string; message: string }
  | { t: "select"; deviceId: string | null }
  | { t: "cmd"; id: string; to?: string; from?: string; cmd: Command }
  | { t: "ack"; id: string; ok: boolean; error?: AckError }
  | {
      t: "rpc";
      id: string;
      to?: string;
      from?: string;
      method: RpcMethod;
      params: Record<string, unknown>;
    }
  | { t: "rpc_result"; id: string; ok: true; result: Record<string, unknown> }
  | { t: "rpc_result"; id: string; ok: false; error: RpcError }
  | { t: "ping"; id: string }
  | { t: "pong"; id: string; serverTime: number };

/* ------------------------------------------------------------- decoding */

type Obj = Record<string, unknown>;

class Invalid extends Error {}

function fail(at: string): never {
  throw new Invalid(at);
}

function obj(v: unknown, at: string): Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Obj) : fail(at);
}

function str(o: Obj, k: string): string {
  const v = o[k];
  return typeof v === "string" ? v : fail(k);
}

/** A nullable field: must be a string or null (missing reads as null). */
function strOrNull(o: Obj, k: string): string | null {
  const v = o[k];
  if (v === null || v === undefined) return null;
  return typeof v === "string" ? v : fail(k);
}

function num(o: Obj, k: string): number {
  const v = o[k];
  return typeof v === "number" && Number.isFinite(v) ? v : fail(k);
}

function numOrNull(o: Obj, k: string): number | null {
  const v = o[k];
  if (v === null || v === undefined) return null;
  return typeof v === "number" && Number.isFinite(v) ? v : fail(k);
}

function bool(o: Obj, k: string): boolean {
  const v = o[k];
  return typeof v === "boolean" ? v : fail(k);
}

function oneOf<T extends string>(v: unknown, allowed: readonly T[], at: string): T {
  return typeof v === "string" && (allowed as readonly string[]).includes(v) ? (v as T) : fail(at);
}

function list<T>(v: unknown, at: string, each: (x: unknown) => T): T[] {
  return Array.isArray(v) ? v.map(each) : fail(at);
}

/** Copy optional key `k` only when present. */
function opt<T>(o: Obj, k: string, read: (o: Obj, k: string) => T): Partial<Record<string, T>> {
  return o[k] === undefined ? {} : { [k]: read(o, k) };
}

const DEVICE_KINDS = ["web", "mobile", "desktop"] as const;
const CAPS = ["player", "yt", "local"] as const;
const REPEAT = ["off", "all", "one"] as const;
const ACK_ERRORS = [
  "offline",
  "not_a_player",
  "nothing_playable",
  "unsupported",
  "failed",
  "timeout",
] as const;
const RPC_ERRORS = ["offline", "unsupported", "not_signed_in", "failed", "timeout"] as const;
const RPC_METHODS = ["search", "suggest", "open", "artist", "library"] as const;
const ITEM_KINDS = ["song", "video", "album", "playlist", "artist"] as const;

function caps(v: unknown): Cap[] {
  return list(v, "caps", (c) => oneOf(c, CAPS, "cap"));
}

export function decodeDeviceInfo(v: unknown): DeviceInfo {
  const o = obj(v, "device");
  return {
    id: str(o, "id"),
    kind: oneOf(o.kind, DEVICE_KINDS, "kind"),
    name: str(o, "name"),
    platform: strOrNull(o, "platform"),
    appVersion: strOrNull(o, "appVersion"),
    caps: caps(o.caps),
    online: bool(o, "online"),
    lastSeenAt: str(o, "lastSeenAt"),
  };
}

function decodeDeviceHello(v: unknown): DeviceHello {
  const o = obj(v, "device");
  return {
    id: str(o, "id"),
    kind: oneOf(o.kind, DEVICE_KINDS, "kind"),
    name: str(o, "name"),
    platform: str(o, "platform"),
    appVersion: str(o, "appVersion"),
    caps: caps(o.caps),
  };
}

export function decodeTrackRef(v: unknown): TrackRef {
  const o = obj(v, "track");
  const common = {
    title: str(o, "title"),
    artist: strOrNull(o, "artist"),
    album: strOrNull(o, "album"),
    durationMs: numOrNull(o, "durationMs"),
  };
  switch (o.src) {
    case "yt":
      return { src: "yt", videoId: str(o, "videoId"), ...common, artUrl: strOrNull(o, "artUrl") };
    case "local":
      return { src: "local", deviceId: str(o, "deviceId"), id: str(o, "id"), ...common, artUrl: null };
    case "other":
      return { src: "other", ...common, artUrl: strOrNull(o, "artUrl") };
    default:
      return fail("src");
  }
}

export function decodeCommand(v: unknown): Command {
  const o = obj(v, "cmd");
  const items = () => list(o.items, "items", decodeTrackRef);
  switch (o.type) {
    case "play":
      return {
        type: "play",
        items: items(),
        index: num(o, "index"),
        ...(opt(o, "positionMs", num) as { positionMs?: number }),
        ...(opt(o, "radio", bool) as { radio?: boolean }),
      };
    case "enqueue":
      return { type: "enqueue", items: items(), mode: oneOf(o.mode, ["next", "end"] as const, "mode") };
    case "pause":
    case "resume":
    case "stop":
    case "next":
    case "prev":
      return { type: o.type };
    case "seek":
      return { type: "seek", positionMs: num(o, "positionMs") };
    case "jump":
      return { type: "jump", uid: str(o, "uid") };
    case "volume":
      return { type: "volume", value: num(o, "value") };
    case "shuffle":
      return { type: "shuffle", on: bool(o, "on") };
    case "repeat":
      return { type: "repeat", mode: oneOf(o.mode, REPEAT, "mode") };
    default:
      return fail("type");
  }
}

export function decodePlayerState(v: unknown): PlayerState {
  const o = obj(v, "state");
  return {
    seq: num(o, "seq"),
    track: o.track === null || o.track === undefined ? null : decodeTrackRef(o.track),
    playing: bool(o, "playing"),
    positionMs: num(o, "positionMs"),
    durationMs: numOrNull(o, "durationMs"),
    volume: num(o, "volume"),
    shuffle: bool(o, "shuffle"),
    repeat: oneOf(o.repeat, REPEAT, "repeat"),
    queue: list(o.queue, "queue", (q) => {
      const e = obj(q, "queue[]");
      return { uid: str(e, "uid"), track: decodeTrackRef(e.track) };
    }),
    ...(opt(o, "at", num) as { at?: number }),
  };
}

export function decodeItem(v: unknown): Item {
  const o = obj(v, "item");
  return {
    kind: oneOf(o.kind, ITEM_KINDS, "kind"),
    id: str(o, "id"),
    title: str(o, "title"),
    subtitle: strOrNull(o, "subtitle"),
    thumbnail: strOrNull(o, "thumbnail"),
    artist: strOrNull(o, "artist"),
    album: strOrNull(o, "album"),
    durationMs: numOrNull(o, "durationMs"),
    hasVideo: bool(o, "hasVideo"),
  };
}

function route(o: Obj): { to?: string; from?: string } {
  return { ...(opt(o, "to", str) as { to?: string }), ...(opt(o, "from", str) as { from?: string }) };
}

function decodeFrameStrict(v: unknown): Frame {
  const o = obj(v, "frame");
  switch (o.t) {
    case "hello":
      return { t: "hello", v: num(o, "v"), token: str(o, "token"), device: decodeDeviceHello(o.device) };
    case "welcome": {
      const states = obj(o.states, "states");
      return {
        t: "welcome",
        v: num(o, "v"),
        deviceId: str(o, "deviceId"),
        serverTime: num(o, "serverTime"),
        devices: list(o.devices, "devices", decodeDeviceInfo),
        activeDeviceId: strOrNull(o, "activeDeviceId"),
        states: Object.fromEntries(
          Object.entries(states).map(([id, s]) => [id, decodePlayerState(s)]),
        ),
      };
    }
    case "devices":
      return {
        t: "devices",
        devices: list(o.devices, "devices", decodeDeviceInfo),
        activeDeviceId: strOrNull(o, "activeDeviceId"),
      };
    case "state":
      return {
        t: "state",
        ...(opt(o, "deviceId", str) as { deviceId?: string }),
        state: decodePlayerState(o.state),
      };
    case "error":
      return { t: "error", code: str(o, "code"), message: str(o, "message") };
    case "select":
      return { t: "select", deviceId: strOrNull(o, "deviceId") };
    case "cmd":
      return { t: "cmd", id: str(o, "id"), ...route(o), cmd: decodeCommand(o.cmd) };
    case "ack":
      return {
        t: "ack",
        id: str(o, "id"),
        ok: bool(o, "ok"),
        ...(o.error === undefined ? {} : { error: oneOf(o.error, ACK_ERRORS, "error") }),
      };
    case "rpc":
      return {
        t: "rpc",
        id: str(o, "id"),
        ...route(o),
        method: oneOf(o.method, RPC_METHODS, "method"),
        params: obj(o.params, "params"),
      };
    case "rpc_result":
      return bool(o, "ok")
        ? { t: "rpc_result", id: str(o, "id"), ok: true, result: obj(o.result, "result") }
        : { t: "rpc_result", id: str(o, "id"), ok: false, error: oneOf(o.error, RPC_ERRORS, "error") };
    case "ping":
      return { t: "ping", id: str(o, "id") };
    case "pong":
      return { t: "pong", id: str(o, "id"), serverTime: num(o, "serverTime") };
    default:
      return fail("t");
  }
}

/** Decode any frame, or `null` when it doesn't match the contract. */
export function decodeFrame(v: unknown): Frame | null {
  try {
    return decodeFrameStrict(v);
  } catch (e) {
    if (e instanceof Invalid) return null;
    throw e;
  }
}

/** Run a decoder, returning `null` instead of throwing on a mismatch. */
export function tryDecode<T>(decode: (v: unknown) => T, v: unknown): T | null {
  try {
    return decode(v);
  } catch (e) {
    if (e instanceof Invalid) return null;
    throw e;
  }
}
