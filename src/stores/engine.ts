import { create } from "zustand";
import {
  autoeqFetchApply,
  chainPresetApply,
  cloudPlay,
  cloudTrackCover,
  cloudTrackTags,
  engineConvolverLoadIr,
  engineEqApplyDdc,
  engineEqImportGraphic,
  engineEqImportVdc,
  engineGetState,
  engineSetBass,
  engineSetCompander,
  engineSetConvolver,
  engineSetEq,
  engineSetMasterVolume,
  engineSetPower,
  engineSetRoom,
  engineSetOutput,
  engineSetSaturation,
  engineSetSpatializer,
  engineSetSurround3d,
  identifyTrack,
  ipcErrorMessage,
  linkPlay,
  linkPlayQueue,
  playerPlayCloudQueue,
  engineSetDataSaver,
  engineSetPlayback,
  playerPause,
  playerPlayFile,
  playerPlayQueue,
  playerPlayRadio,
  playerPlayYtmusicQueue,
  playerResume,
  playerSeek,
  playerStop,
  profileClear,
  ytmusicPlay,
  ytmusicPrefetch,
} from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { BAND_COUNT } from "@/lib/types";
import {
  observe,
  effectiveMode,
  chooseStreamMode,
  loadNetworkMode,
  saveNetworkMode,
} from "./networkMode";
import type { NetworkState } from "./networkMode";
import type {
  CloudEntry,
  CloudTrackMeta,
  CompanderState,
  ConvolverState,
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  LibraryTrack,
  MeterFrame,
  PhoneDevice,
  OutputState,
  PhoneTrack,
  RadioStation,
  RoomState,
  SaturationState,
  SpatialMode,
  Surround3DState,
  TrackMeta,
  TransportProgress,
  YtTrack,
} from "@/lib/types";

/** How the engine reaches the audio for a queued item. */
export type QueueSource = "local" | "phone" | "cloud" | "ytmusic" | "radio" | "cast";

/** Auto-advance behaviour at the end of a track. */
export type RepeatMode = "off" | "all" | "one";

/**
 * One entry in the unified playback queue. A queue is always built from a
 * single source (a library list, a phone's library, or one cloud folder), so
 * every item shares the same `source`.
 */
export interface QueueItem {
  /** Stable id for highlight/dedup: local path | phone/cloud id | url. */
  id: string;
  source: QueueSource;
  title: string;
  artist: string | null;
  album: string | null;
  durationSecs: number | null;
  /** Resolved cover (a `data:` URI), e.g. a cloud track's art once its tags
   *  load. Local/phone items resolve art lazily from their path instead. */
  cover?: string | null;
  // Exactly one payload is set, matching `source`:
  track?: LibraryTrack;
  device?: PhoneDevice;
  phoneTrack?: PhoneTrack;
  cloud?: CloudEntry;
  ytTrack?: YtTrack;
  radioUrl?: string;
}

const defaultEngineState: EngineState = {
  power: true,
  masterVolume: 1,
  eq: { enabled: true, preGain: 0, bands: Array<number>(BAND_COUNT).fill(0) },
  bass: { enabled: false, amount: 0, harmonics: false, adaptive: false },
  spatializer: { enabled: false, amount: 0.5, mode: "crossfeed" },
  surround3d: {
    enabled: false,
    intensity: 0.5,
    subwoofer: 0.25,
    speakers: {
      frontL: true,
      frontR: true,
      sideL: true,
      sideR: true,
      surroundL: true,
      surroundR: true,
    },
  },
  room: {
    enabled: false,
    roomSize: 0.4,
    decay: 0.4,
    damping: 0.45,
    preDelay: 8,
    diffusion: 0.55,
    wetDry: 0.3,
    activePresetId: null,
  },
  convolver: {
    enabled: false,
    wetDry: 1.0,
    irGainDb: 0.0,
    irId: null,
    irName: null,
    irSeconds: 0.0,
    irTruncated: false,
  },
  compander: {
    enabled: false,
    thresholdDb: -18.0,
    ratio: 2.5,
    kneeDb: 8.0,
    attackMs: 15.0,
    releaseMs: 45.0,
    makeupDb: 0.0,
    gateDb: -70.0,
    expanderRatio: 2.0,
  },
  saturation: { enabled: false, drive: 0.3, mix: 1.0 },
  script: { enabled: false, source: "" },
  headphone: { enabled: false, preamp: 0, bands: [] },
  output: { gainDb: 0, limiterEnabled: true, ceilingDb: -0.3 },
  playback: { gapless: true, crossfadeSecs: 0, dataSaver: false },
  activePresetId: null,
  activeProfileId: null,
};

const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

// A single shared "no gain reduction" compander meter row. Re-used as a stable
// reference across idle frames so the meter doesn't re-render 30×/s for zeros.
const IDLE_COMPANDER_GR: number[] = new Array<number>(10).fill(0);

/* --------------------------------------------------------------- queue items */

export function localItem(track: LibraryTrack): QueueItem {
  return {
    id: track.path,
    source: "local",
    title: track.title,
    artist: track.artist,
    album: track.album,
    durationSecs: track.durationSecs,
    track,
  };
}

export function phoneItem(device: PhoneDevice, t: PhoneTrack): QueueItem {
  return {
    id: t.id,
    source: "phone",
    title: t.title,
    artist: t.artist,
    album: t.album,
    durationSecs: t.durationMs != null ? t.durationMs / 1000 : null,
    device,
    phoneTrack: t,
  };
}

export function cloudItem(file: CloudEntry): QueueItem {
  return {
    id: file.id,
    source: "cloud",
    title: file.name,
    artist: null,
    album: null,
    durationSecs: null,
    cloud: file,
  };
}

export function ytmusicItem(t: YtTrack): QueueItem {
  return {
    id: t.videoId,
    source: "ytmusic",
    title: t.title,
    artist: t.artist,
    album: t.album,
    durationSecs: t.durationSecs,
    // The API hands us the thumbnail with the listing, so the card has its art
    // from the start — no lazy per-track cover fetch like cloud needs.
    cover: t.thumbnail,
    ytTrack: t,
  };
}

/** Extension hint from a cloud file name (e.g. "Song.flac" → "flac"), for the
 *  demuxer. Returns undefined when there's no extension. */
function extFromName(name: string): string | undefined {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : undefined;
}

/** The now-playing card's view of a queue item, before the engine has decoded
 *  anything from the file itself.
 *
 *  `cover` carries the item's own art when it has any. YT Music hands us a
 *  thumbnail with the listing, and hardcoding null here meant the card waited
 *  forever for art the engine could never supply: googlevideo streams a bare
 *  DASH audio track with no embedded tags, so the sidebar sat on its gradient
 *  while the very same cover showed in every list. Cloud items benefit too — a
 *  preloaded cover shows immediately and saves the lazy fetch. Local files carry
 *  none here and still get theirs decoded, exactly as before. */
export function itemMeta(item: QueueItem): TrackMeta {
  return {
    title: item.title,
    artist: item.artist,
    album: item.album,
    cover: item.cover ?? null,
  };
}

/* ----------------------------------------------------------------- play order */

/** Fisher–Yates shuffle (in place). */
function shuffleInPlace<T>(arr: T[]): T[] {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [arr[i], arr[j]] = [arr[j]!, arr[i]!];
  }
  return arr;
}

/** Build a play order over `len` items, keeping `start` first when shuffling. */
function buildOrder(
  len: number,
  shuffle: boolean,
  start: number,
): { order: number[]; orderPos: number } {
  const identity = Array.from({ length: len }, (_, i) => i);
  if (!shuffle || len <= 1) return { order: identity, orderPos: start };
  const rest = identity.filter((i) => i !== start);
  shuffleInPlace(rest);
  return { order: [start, ...rest], orderPos: 0 };
}

/** Next order position in `dir`, honouring repeat; null when there's nowhere to go. */
function stepOrder(
  pos: number,
  len: number,
  repeat: RepeatMode,
  dir: 1 | -1,
): number | null {
  if (len === 0 || pos < 0) return null;
  const n = pos + dir;
  if (n >= 0 && n < len) return n;
  if (repeat === "all") return ((n % len) + len) % len;
  return null;
}

interface EngineStore {
  state: EngineState;
  meters: MeterFrame;
  spectrum: number[];
  /** Per-band compander gain-reduction in dB (10 values, ≤0). Zeros when idle. */
  companderGr: number[];
  metersLive: boolean;

  // transport
  playing: boolean;
  paused: boolean;
  nowPlaying: string | null;
  /** Rich now-playing metadata (tags + cover) for the docked bar. */
  nowPlayingMeta: TrackMeta | null;
  /** The current track's local file path (null for phone/cloud/radio). */
  currentTrackPath: string | null;
  positionSecs: number;
  durationSecs: number | null;
  /** Whether the active source can be scrubbed (false for live radio). */
  seekable: boolean;
  /** True while the engine is rebuffering/stalled. */
  buffering: boolean;

  // queue
  queue: QueueItem[];
  /** Index into `queue` of the current item (= order[orderPos]); -1 when idle. */
  queueIndex: number;
  /** Play order over the queue (identity, or shuffled). */
  order: number[];
  /** Position within `order`; -1 when idle. */
  orderPos: number;
  shuffle: boolean;
  repeat: RepeatMode;

  hydrate: (state: EngineState) => void;
  setPower: (power: boolean) => void;
  setMasterVolume: (v: number) => void;
  setBand: (index: number, valueDb: number) => void;
  setBands: (bands: number[]) => void;
  setPreGain: (preGain: number) => void;
  setEqEnabled: (enabled: boolean) => void;
  applyPreset: (preset: EqPreset) => void;
  setBass: (enabled: boolean, amount: number, harmonics: boolean, adaptive: boolean) => void;
  setSpatializer: (enabled: boolean, amount: number, mode: SpatialMode) => void;
  setSurround3d: (next: Surround3DState) => void;
  setRoom: (next: RoomState) => void;
  setConvolver: (next: ConvolverState) => void;
  setCompander: (next: CompanderState) => void;
  setSaturation: (next: SaturationState) => void;
  setOutput: (next: OutputState) => void;
  loadConvolverIr: (path: string) => Promise<void>;
  /** Import an EqualizerAPO GraphicEQ curve. Throws on IPC failure — caller must catch. */
  importGraphicEq: (curve: string) => Promise<void>;
  /** Import a ViPER/JamesDSP DDC (.vdc) file by path. Throws on failure — caller must catch. */
  importVdc: (path: string) => Promise<void>;
  /** Apply a bundled ViPER DDC preset by name. Throws on failure — caller must catch. */
  applyDdc: (name: string) => Promise<void>;
  /** Fetch a model's AutoEq curve from the bundled index URL and apply it.
   *  Throws on failure — caller must catch. */
  applyAutoEq: (url: string) => Promise<void>;
  applyProfile: (profile: HeadphoneProfile) => void;
  clearProfile: () => void;
  setPlayback: (gapless: boolean, crossfadeSecs: number) => void;
  setDataSaver: (on: boolean) => void;

  applyFrame: (frame: EngineFrame) => void;
  applyProgress: (p: TransportProgress) => void;
  setPlaying: (playing: boolean) => void;
  /** Merge decoded engine metadata (tags + cover) into the now-playing card. */
  applyNowPlaying: (meta: TrackMeta) => void;
  /** Follow the gapless queue's current track index (from the engine). */
  applyQueueIndex: (index: number) => void;

  /** Play an ad-hoc file (single-item queue). Throws on IPC error. */
  play: (path: string, name: string) => Promise<void>;
  /** Play a local track list starting at `index`. */
  playFromList: (tracks: LibraryTrack[], index: number) => void;
  /** Play a phone's track list starting at `index`. */
  playPhoneList: (device: PhoneDevice, tracks: PhoneTrack[], index: number) => void;
  /** Play a list of cloud files starting at `index`. */
  playCloudList: (files: CloudEntry[], index: number) => void;
  /** Play a pre-built queue of items (any mix of sources) starting at `index`. */
  playQueueItems: (items: QueueItem[], index: number) => void;
  /** Jump to a position in the current play order. */
  jumpTo: (orderPos: number) => void;
  /** Remove an item (by its queue index) from the current queue. */
  removeFromQueue: (queueIndex: number) => void;
  /** Stream an internet radio station (live; no queue/duration). */
  playRadio: (station: RadioStation) => void;
  /** Stream a single cloud file (folder context unknown). */
  playCloud: (file: CloudEntry) => void;
  /** Stream a single track from a paired phone. */
  playPhone: (device: PhoneDevice, track: PhoneTrack) => void;
  /** Reflect a track a phone has cast to this desktop (started server-side). */
  castIncoming: (title: string, artist: string | null) => void;
  next: () => void;
  prev: () => void;
  togglePause: () => void;
  seek: (secs: number) => void;
  stop: () => Promise<void>;
  toggleShuffle: () => void;
  cycleRepeat: () => void;
  /** Apply a transport action from the OS media controls. */
  handleMediaCommand: (action: string, secs: number | null) => void;
  /** Fingerprint the current local track and fill any missing tags in place,
   *  reflecting recognized title/artist/album in the now-playing card. */
  identifyNowPlaying: () => Promise<void>;
  /** Apply a whole-chain preset by id, then re-hydrate the UI from the engine. */
  applyChainPreset: (id: string) => Promise<void>;
}

export const useEngineStore = create<EngineStore>((set, get) => {
  // Not part of rendered state: distinguishes user-stop from natural end.
  let userStopped = false;
  // Whether the engine is currently driving a multi-track gapless queue (true),
  // versus a single track / stream the store advances itself (false). Decides
  // how a natural end-of-stream is interpreted.
  let gaplessQueueRunning = false;
  // Persisted across queues and restarts so a proven-slow link stays on the
  // single-track path and a proven-fast one keeps crossfading; an unmeasured
  // link starts "unknown", which chooseStreamMode treats optimistically.
  // Read through `effectiveMode` at each decision, never cached as a verdict:
  // a "constrained" that is only aged at startup can never expire within the
  // session it is punishing.
  let networkState: NetworkState = loadNetworkMode();
  let lastRebuffer = 0;

  const pushEq = (eq: EngineState["eq"]) => {
    void engineSetEq(eq.bands, eq.preGain, eq.enabled).catch(() => {});
  };

  /** Reset all transport/queue state to idle. */
  const idleState = () => ({
    playing: false,
    paused: false,
    metersLive: false,
    meters: idleMeters,
    nowPlaying: null,
    nowPlayingMeta: null,
    currentTrackPath: null,
    positionSecs: 0,
    durationSecs: null,
    seekable: false,
    buffering: false,
  });

  /**
   * Start the item at order position `pos`: set the now-playing card and tell
   * the engine to play it. Local lists use the engine's gapless queue (so they
   * play back-to-back with no gap); phone/cloud/radio play one stream at a time.
   */
  const startPlayback = (pos: number) => {
    const { queue, order, repeat, state } = get();
    const qi = order[pos];
    const item = qi != null ? queue[qi] : undefined;
    if (!item) return;

    set({
      queueIndex: qi!,
      orderPos: pos,
      nowPlaying: item.title,
      nowPlayingMeta: itemMeta(item),
      // Only local files can be stem-separated.
      currentTrackPath: item.source === "local" ? (item.track?.path ?? null) : null,
      playing: true,
      paused: false,
      positionSecs: 0,
      durationSecs: item.durationSecs,
      // Optimistic; the backend's progress events correct this per source.
      seekable: item.source === "local",
      buffering: false,
    });

    const onError = (e: unknown) =>
      toast.error(`Couldn't play ${item.title}: ${ipcErrorMessage(e)}`);

    const { gapless, crossfadeSecs, dataSaver } = state.playback;
    // The engine's gapless/crossfade queue needs a homogeneous source: an all-
    // local, all-cloud (same account), all-phone (same device), or all-YT Music
    // queue. A cloud queue resolves URLs with one account's tokens, so accounts
    // can't be mixed in it; mixed queues advance track-by-track from the store
    // instead. YT Music has a single signed-in account, so it needs no such
    // same-account constraint — being all-ytmusic is enough.
    const wantQueue = (gapless || crossfadeSecs > 0) && repeat !== "one";
    const allLocal = order.every((i) => queue[i]?.source === "local");
    const allCloud =
      order.every((i) => queue[i]?.cloud != null) &&
      order.every((i) => queue[i]?.cloud?.accountId === item.cloud?.accountId);
    const allPhone =
      order.every((i) => queue[i]?.phoneTrack != null) &&
      order.every((i) => queue[i]?.device?.id === item.device?.id);
    const allYtMusic = order.every((i) => queue[i]?.ytTrack != null);

    const streamMode =
      item.source === "cloud" || item.source === "phone" || item.source === "ytmusic"
        ? chooseStreamMode(item.source, dataSaver, effectiveMode(networkState, Date.now()))
        : "progressive";
    const useEngineQueue = item.source === "local" && allLocal && wantQueue;
    const useCloudQueue =
      item.source === "cloud" && allCloud && wantQueue && streamMode === "gapless";
    const usePhoneQueue =
      item.source === "phone" && allPhone && wantQueue && streamMode === "gapless";
    const useYtMusicQueue =
      item.source === "ytmusic" && allYtMusic && wantQueue && streamMode === "gapless";

    gaplessQueueRunning =
      useEngineQueue || useCloudQueue || usePhoneQueue || useYtMusicQueue;

    switch (item.source) {
      case "local":
        if (useEngineQueue) {
          const paths = order
            .map((i) => queue[i]?.track?.path)
            .filter((p): p is string => typeof p === "string");
          void playerPlayQueue(paths, pos).catch(onError);
        } else {
          // Single track (gapless off, or repeat-one): the store advances.
          void playerPlayFile(item.track!.path).catch(onError);
        }
        break;
      case "phone":
        if (usePhoneQueue) {
          const items = order.map((i) => ({
            id: queue[i]!.phoneTrack!.id,
            ext: queue[i]!.phoneTrack!.ext,
          }));
          void linkPlayQueue(item.device!.id, items, pos).catch(onError);
        } else {
          void linkPlay(
            item.device!.id,
            item.phoneTrack!.id,
            item.phoneTrack!.ext,
            item.durationSecs,
          ).catch(onError);
        }
        break;
      case "cloud":
        if (useCloudQueue) {
          const items = order.map((i) => ({
            id: queue[i]!.cloud!.id,
            ext: extFromName(queue[i]!.cloud!.name),
          }));
          void playerPlayCloudQueue(item.cloud!.accountId, items, pos).catch(onError);
        } else {
          void cloudPlay(item.cloud!.accountId, item.cloud!.id).catch(onError);
        }
        break;
      case "ytmusic":
        if (useYtMusicQueue) {
          const items = order.map((i) => ({ videoId: queue[i]!.ytTrack!.videoId }));
          void playerPlayYtmusicQueue(items, pos).catch(onError);
        } else {
          void ytmusicPlay(item.ytTrack!.videoId, item.durationSecs).catch(onError);
          // Resolve the next track while this one plays. Without it the whole
          // ~5s yt-dlp resolve happens after the current track ends, because
          // that's the first moment anything asks for the next url — and every
          // second of it is silence. The gapless queue has its own lookahead;
          // this path had none.
          const nextPos = stepOrder(pos, order.length, repeat, 1);
          const next = nextPos === null ? undefined : queue[order[nextPos]!];
          if (next?.ytTrack) void ytmusicPrefetch(next.ytTrack.videoId).catch(() => {});
        }
        break;
      case "radio":
        void playerPlayRadio(item.radioUrl!).catch(onError);
        break;
      case "cast":
        break; // already playing on the casting phone
    }

    // Cloud covers aren't stored on queue items; fetch the current one lazily.
    fillNowPlayingCover(item);
  };

  /** A cloud item still labelled with its file name (or no artist) needs its
   *  real tags fetched. Items enqueued from the library already carry them. */
  const cloudNeedsMeta = (it: QueueItem): boolean => {
    if (it.source !== "cloud" || !it.cloud) return false;
    if (!it.artist || it.artist.trim() === "") return true;
    const stem = it.cloud.name.replace(/\.[^./\\]+$/, "");
    return it.title === it.cloud.name || it.title === stem;
  };

  /** Lazily resolve the *current* cloud track's cover into the now-playing
   *  card (one fetch per track change). Covers are never bulk-loaded onto
   *  queue items — the engine's decoded `now_playing` cover still wins if it
   *  lands first, and the backend caches per file so repeats are cheap. */
  const fillNowPlayingCover = (item: QueueItem): void => {
    if (item.source !== "cloud" || !item.cloud || item.cover) return;
    const file = item.cloud;
    void cloudTrackCover(file.accountId, file.id, file.name)
      .then((cover) => {
        if (!cover) return;
        set((s) => {
          // Only if this is still the current track and no decoded cover
          // arrived from the engine in the meantime.
          const cur = s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined;
          if (!cur || cur.source !== "cloud" || cur.id !== item.id) return {};
          if (!s.nowPlayingMeta || s.nowPlayingMeta.cover) return {};
          return { nowPlayingMeta: { ...s.nowPlayingMeta, cover } };
        });
      })
      .catch(() => {});
  };

  /** How often resolved cloud tags are flushed into the queue (one store
   *  update per flush, instead of one per track). */
  const ENRICH_FLUSH_MS = 200;

  /** Background-fill missing cloud tags for a queue (cache-backed, bounded), so
   *  the queue list shows real title/artist instead of the file name. Tags
   *  only — never the ~100 KB base64 cover, so a 10k-track queue doesn't hold
   *  ~1 GB of covers in memory; covers resolve lazily per visible queue row
   *  (and via {@link fillNowPlayingCover} for the current track). Resolved
   *  tags are batched and applied as one new queue array per flush. */
  const enrichCloudQueue = (items: QueueItem[]): void => {
    const pending = items.filter(cloudNeedsMeta);
    if (pending.length === 0) return;
    let next = 0;
    // Tags resolved but not yet applied to the store, keyed by queue item id.
    let resolved = new Map<string, CloudTrackMeta>();
    let flushTimer: number | null = null;

    const flush = () => {
      flushTimer = null;
      if (resolved.size === 0) return;
      const batch = resolved;
      resolved = new Map();
      set((s) => {
        // The queue may have been reordered/replaced since these resolved:
        // map ids → current indexes once per flush (not a findIndex each).
        const indexById = new Map<string, number>();
        for (let i = 0; i < s.queue.length; i++) {
          const q = s.queue[i]!;
          if (q.source === "cloud") indexById.set(q.id, i);
        }
        let queue: QueueItem[] | null = null;
        let nowPatch: Partial<
          Pick<EngineStore, "nowPlaying" | "nowPlayingMeta">
        > = {};
        for (const [id, meta] of batch) {
          const idx = indexById.get(id);
          if (idx == null) continue;
          const cur = (queue ?? s.queue)[idx]!;
          const title = meta.title ?? cur.title;
          const artist = meta.artist ?? cur.artist;
          const album = meta.album ?? cur.album;
          if (title === cur.title && artist === cur.artist && album === cur.album) {
            continue;
          }
          queue ??= s.queue.slice();
          queue[idx] = { ...cur, title, artist, album };
          // Keep the now-playing card in sync when the current track's tags land.
          if (s.queueIndex === idx) {
            nowPatch = {
              nowPlaying: title,
              nowPlayingMeta: {
                title,
                artist,
                album,
                cover: s.nowPlayingMeta?.cover ?? null,
              },
            };
          }
        }
        return queue ? { queue, ...nowPatch } : {};
      });
    };

    const scheduleFlush = () => {
      if (flushTimer == null) flushTimer = window.setTimeout(flush, ENRICH_FLUSH_MS);
    };

    const worker = async () => {
      while (next < pending.length) {
        const it = pending[next++]!;
        const file = it.cloud!;
        try {
          const meta = await cloudTrackTags(file.accountId, file.id, file.name);
          if (meta && (meta.title || meta.artist || meta.album)) {
            resolved.set(it.id, meta);
            scheduleFlush();
          }
        } catch {
          // Skip — one failed lookup shouldn't block the rest.
        }
      }
    };
    void Promise.all([worker(), worker(), worker(), worker()]).then(() => {
      // Trailing flush for whatever resolved after the last timer fired.
      if (flushTimer != null) window.clearTimeout(flushTimer);
      flush();
    });
  };

  /** Replace the queue and start playing `index`. */
  const setQueueAndPlay = (items: QueueItem[], index: number) => {
    if (items.length === 0) return;
    const start = Math.max(0, Math.min(index, items.length - 1));
    const { order, orderPos } = buildOrder(items.length, get().shuffle, start);
    // Carry the network classification across queues (persisted) instead of
    // resetting to "unknown" — a fresh reset made every queue re-measure from
    // scratch, and since the routing decision is taken at play-start, before
    // any sample exists, cloud/phone queues always fell to the single-track
    // (no-crossfade) path. Only the per-stream rebuffer counter is reset.
    lastRebuffer = 0;
    set({ queue: items, order, orderPos, queueIndex: order[orderPos]! });
    startPlayback(orderPos);
    enrichCloudQueue(items);
  };

  /** Decide what to play when the current item ends naturally. */
  const advanceOnEnd = () => {
    const { queue, order, orderPos, repeat } = get();
    const item = orderPos >= 0 ? queue[order[orderPos]!] : undefined;
    if (!item || item.source === "cast" || item.source === "radio") {
      set(idleState());
      return;
    }
    if (repeat === "one") {
      startPlayback(orderPos);
      return;
    }
    if (gaplessQueueRunning) {
      // The whole local gapless list just finished.
      if (repeat === "all" && order.length > 0) startPlayback(0);
      else set(idleState());
      return;
    }
    // Single-track sources (phone/cloud/non-gapless local): advance by one.
    const np = stepOrder(orderPos, order.length, repeat, 1);
    if (np !== null) startPlayback(np);
    else set(idleState());
  };

  return {
    state: defaultEngineState,
    meters: idleMeters,
    spectrum: [],
    companderGr: IDLE_COMPANDER_GR,
    metersLive: false,
    playing: false,
    paused: false,
    nowPlaying: null,
    nowPlayingMeta: null,
    currentTrackPath: null,
    positionSecs: 0,
    durationSecs: null,
    seekable: false,
    buffering: false,
    queue: [],
    queueIndex: -1,
    order: [],
    orderPos: -1,
    shuffle: false,
    repeat: "off",

    hydrate: (state) => set({ state }),

    setPower: (power) => {
      set((s) => ({ state: { ...s.state, power } }));
      void engineSetPower(power).catch(() => {});
    },
    setMasterVolume: (masterVolume) => {
      set((s) => ({ state: { ...s.state, masterVolume } }));
      void engineSetMasterVolume(masterVolume).catch(() => {});
    },
    setBand: (index, valueDb) => {
      const bands = get().state.eq.bands.slice();
      bands[index] = valueDb;
      const nextEq = { ...get().state.eq, bands };
      set((s) => ({ state: { ...s.state, eq: nextEq, activePresetId: null } }));
      pushEq(nextEq);
    },
    setBands: (bands) => {
      const nextEq = { ...get().state.eq, bands: bands.slice() };
      set((s) => ({ state: { ...s.state, eq: nextEq, activePresetId: null } }));
      pushEq(nextEq);
    },
    setPreGain: (preGain) => {
      const nextEq = { ...get().state.eq, preGain };
      set((s) => ({ state: { ...s.state, eq: nextEq } }));
      pushEq(nextEq);
    },
    setEqEnabled: (enabled) => {
      const nextEq = { ...get().state.eq, enabled };
      set((s) => ({ state: { ...s.state, eq: nextEq } }));
      pushEq(nextEq);
    },
    applyPreset: (preset) =>
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, bands: preset.bands.slice(), preGain: preset.preGain },
          activePresetId: preset.id,
        },
      })),
    setBass: (enabled, amount, harmonics, adaptive) => {
      set((s) => ({ state: { ...s.state, bass: { enabled, amount, harmonics, adaptive } } }));
      void engineSetBass(enabled, amount, harmonics, adaptive).catch(() => {});
    },
    setSpatializer: (enabled, amount, mode) => {
      set((s) => ({ state: { ...s.state, spatializer: { enabled, amount, mode } } }));
      void engineSetSpatializer(enabled, amount, mode).catch(() => {});
    },
    setSurround3d: (next) => {
      set((s) => ({ state: { ...s.state, surround3d: next } }));
      void engineSetSurround3d(
        next.enabled,
        next.intensity,
        next.subwoofer,
        next.speakers,
      ).catch(() => {});
    },
    setRoom: (next) => {
      set((s) => ({ state: { ...s.state, room: next } }));
      void engineSetRoom(next).catch(() => {});
    },
    setConvolver: (next) => {
      set((s) => ({ state: { ...s.state, convolver: next } }));
      void engineSetConvolver(next).catch(() => {});
    },
    setCompander: (next) => {
      set((s) => ({ state: { ...s.state, compander: next } }));
      void engineSetCompander(next).catch(() => {});
    },
    setSaturation: (next) => {
      set((s) => ({ state: { ...s.state, saturation: next } }));
      void engineSetSaturation(next).catch(() => {});
    },
    setOutput: (next) => {
      set((s) => ({ state: { ...s.state, output: next } }));
      void engineSetOutput(next).catch(() => {});
    },
    loadConvolverIr: async (path) => {
      const info = await engineConvolverLoadIr(path);
      set((s) => ({
        state: {
          ...s.state,
          convolver: {
            ...s.state.convolver,
            enabled: true,
            irId: path,
            irName: info.name,
            irSeconds: info.seconds,
            irTruncated: info.truncated,
          },
        },
      }));
    },
    importGraphicEq: async (curve) => {
      const res = await engineEqImportGraphic(curve);
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, enabled: true, bands: res.bands, preGain: res.preGain },
          activePresetId: null,
        },
      }));
    },
    importVdc: async (path) => {
      const res = await engineEqImportVdc(path);
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, enabled: true, bands: res.bands, preGain: res.preGain },
          activePresetId: null,
        },
      }));
    },
    applyDdc: async (name) => {
      const res = await engineEqApplyDdc(name);
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, enabled: true, bands: res.bands, preGain: res.preGain },
          activePresetId: null,
        },
      }));
    },
    applyAutoEq: async (url) => {
      const res = await autoeqFetchApply(url);
      set((s) => ({
        state: {
          ...s.state,
          eq: { ...s.state.eq, enabled: true, bands: res.bands, preGain: res.preGain },
          activePresetId: null,
        },
      }));
    },
    applyProfile: (profile) =>
      set((s) => ({
        state: {
          ...s.state,
          headphone: { enabled: true, preamp: profile.preamp, bands: profile.bands },
          activeProfileId: profile.id,
        },
      })),
    clearProfile: () => {
      set((s) => ({
        state: {
          ...s.state,
          headphone: { enabled: false, preamp: 0, bands: [] },
          activeProfileId: null,
        },
      }));
      void profileClear().catch(() => {});
    },

    setPlayback: (gapless, crossfadeSecs) => {
      set((s) => ({ state: { ...s.state, playback: { ...s.state.playback, gapless, crossfadeSecs } } }));
      void engineSetPlayback(gapless, crossfadeSecs).catch(() => {});
    },

    setDataSaver: (on) => {
      set((s) => ({ state: { ...s.state, playback: { ...s.state.playback, dataSaver: on } } }));
      void engineSetDataSaver(on).catch(() => {});
    },

    applyFrame: (frame) =>
      set((s) => {
        // Keep a *stable* companderGr reference whenever there's no gain
        // reduction (stage idle/disabled): re-using the same frozen array means
        // the CompanderCard meter's `s.companderGr` selector stays reference-
        // equal and it doesn't re-render every frame for a row of zeros.
        const gr = frame.companderGr;
        const active = gr != null && gr.some((v) => v !== 0);
        const companderGr = active
          ? gr
          : s.companderGr === IDLE_COMPANDER_GR
            ? s.companderGr
            : IDLE_COMPANDER_GR;
        return {
          meters: frame.meters,
          ...(frame.spectrum ? { spectrum: frame.spectrum } : {}),
          companderGr,
        };
      }),

    applyProgress: (p) => {
      const delta = Math.max(0, p.rebufferCount - lastRebuffer);
      lastRebuffer = p.rebufferCount;
      const prevMode = networkState.mode;
      networkState = observe(
        networkState,
        { downloadBps: p.downloadBps, rebufferDelta: delta },
        Date.now(),
      );
      if (networkState.mode !== prevMode) saveNetworkMode(networkState);
      set((s) => ({
        positionSecs: p.positionSecs,
        // Keep a known (item-provided) duration until the backend learns one,
        // so streams show their length before the first progress with a value.
        durationSecs: p.durationSecs ?? s.durationSecs,
        paused: p.paused,
        seekable: p.seekable,
        buffering: p.buffering,
      }));
    },

    setPlaying: (playing) => {
      if (playing) {
        set({ playing: true, metersLive: true });
        return;
      }
      // Stopped/ended.
      if (userStopped) {
        userStopped = false;
        set(idleState());
        return;
      }
      advanceOnEnd();
    },

    applyNowPlaying: (meta) =>
      set((s) => {
        // Ignore late events after playback stopped.
        if (!s.nowPlaying && !s.playing) return {};
        const prev = s.nowPlayingMeta;
        const nowPlayingMeta = {
          title: meta.title ?? prev?.title ?? s.nowPlaying,
          artist: meta.artist ?? prev?.artist ?? null,
          album: meta.album ?? prev?.album ?? null,
          cover: meta.cover ?? prev?.cover ?? null,
        };
        // Mirror the stream-resolved tags onto the current queue item so the
        // queue list matches the now-playing bar — cloud tracks are queued under
        // their file name until their real tags arrive from the stream.
        let queue = s.queue;
        const qi = s.queueIndex;
        const cur = qi >= 0 ? s.queue[qi] : undefined;
        if (cur && (meta.title || meta.artist || meta.album || meta.cover)) {
          const patched: QueueItem = {
            ...cur,
            title: meta.title ?? cur.title,
            artist: meta.artist ?? cur.artist,
            album: meta.album ?? cur.album,
            cover: meta.cover ?? cur.cover ?? null,
          };
          if (
            patched.title !== cur.title ||
            patched.artist !== cur.artist ||
            patched.album !== cur.album ||
            patched.cover !== cur.cover
          ) {
            queue = s.queue.slice();
            queue[qi] = patched;
          }
        }
        return {
          nowPlayingMeta,
          ...(queue !== s.queue ? { queue } : {}),
          // Keep the title string in sync for views that match on it.
          ...(meta.title ? { nowPlaying: meta.title } : {}),
        };
      }),

    applyQueueIndex: (absPos) => {
      // Only the engine's gapless queue emits this; map its absolute index back
      // to our order position. Ignored for single-track / streamed playback.
      if (!gaplessQueueRunning) return;
      const { order, queue } = get();
      const qi = order[absPos];
      const item = qi != null ? queue[qi] : undefined;
      if (!item) return;
      set({
        orderPos: absPos,
        queueIndex: qi!,
        nowPlaying: item.title,
        nowPlayingMeta: itemMeta(item),
        positionSecs: 0,
        durationSecs: item.durationSecs,
      });
      // Cloud covers aren't stored on queue items; fetch the current one lazily.
      fillNowPlayingCover(item);
    },

    play: async (path, name) => {
      const items = [
        localItem({ path, title: name, artist: null, album: null, genre: null, durationSecs: null }),
      ];
      setQueueAndPlay(items, 0);
    },

    playFromList: (tracks, index) =>
      setQueueAndPlay(tracks.map(localItem), index),

    playPhoneList: (device, tracks, index) =>
      setQueueAndPlay(
        tracks.map((t) => phoneItem(device, t)),
        index,
      ),

    playCloudList: (files, index) => {
      const audio = files.filter((f) => !f.isFolder);
      if (audio.length === 0) return;
      // Re-base the index onto the filtered (audio-only) list.
      const target = files[index];
      const start = target ? Math.max(0, audio.findIndex((f) => f.id === target.id)) : 0;
      setQueueAndPlay(audio.map(cloudItem), start);
    },

    playQueueItems: (items, index) => setQueueAndPlay(items, index),

    jumpTo: (pos) => {
      const { order } = get();
      if (pos >= 0 && pos < order.length) startPlayback(pos);
    },

    removeFromQueue: (qIndex) => {
      const { queue, order, orderPos } = get();
      if (qIndex < 0 || qIndex >= queue.length) return;
      const currentQ = orderPos >= 0 ? order[orderPos]! : -1;
      const newQueue = queue.filter((_, i) => i !== qIndex);
      if (newQueue.length === 0) {
        void get().stop();
        return;
      }
      // Drop the index from the order and renumber entries past it.
      const newOrder = order
        .filter((i) => i !== qIndex)
        .map((i) => (i > qIndex ? i - 1 : i));
      if (qIndex === currentQ) {
        // Removed the playing track: play whatever takes its order slot.
        const pos = Math.min(orderPos, newOrder.length - 1);
        set({ queue: newQueue, order: newOrder });
        startPlayback(pos);
      } else {
        const newCur = currentQ > qIndex ? currentQ - 1 : currentQ;
        const newPos = newOrder.indexOf(newCur);
        set({ queue: newQueue, order: newOrder, orderPos: newPos, queueIndex: newCur });
        // Keep the engine's gapless list in sync (re-issues from current).
        if (gaplessQueueRunning) startPlayback(newPos);
      }
    },

    playRadio: (station) => {
      const item: QueueItem = {
        id: station.url,
        source: "radio",
        title: station.name,
        artist: null,
        album: null,
        durationSecs: null,
        radioUrl: station.url,
      };
      gaplessQueueRunning = false;
      set({ queue: [item], order: [0], orderPos: 0, queueIndex: 0 });
      startPlayback(0);
    },

    playCloud: (file) => setQueueAndPlay([cloudItem(file)], 0),

    playPhone: (device, track) => setQueueAndPlay([phoneItem(device, track)], 0),

    castIncoming: (title, artist) => {
      const item: QueueItem = {
        id: title,
        source: "cast",
        title,
        artist,
        album: null,
        durationSecs: null,
      };
      gaplessQueueRunning = false;
      set({
        queue: [item],
        order: [0],
        orderPos: 0,
        queueIndex: 0,
        nowPlaying: title,
        nowPlayingMeta: itemMeta(item),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: null,
        seekable: false,
        buffering: false,
      });
    },

    next: () => {
      const { order, orderPos, repeat, queue } = get();
      const item = orderPos >= 0 ? queue[order[orderPos]!] : undefined;
      if (!item || item.source === "cast") return;
      const np = stepOrder(orderPos, order.length, repeat, 1);
      if (np !== null) startPlayback(np);
    },

    prev: () => {
      const { order, orderPos, repeat, queue, positionSecs, seekable } = get();
      const item = orderPos >= 0 ? queue[order[orderPos]!] : undefined;
      if (!item || item.source === "cast") return;
      // Restart the current track if we're a few seconds in (familiar UX).
      if (positionSecs > 3 && seekable) {
        get().seek(0);
        return;
      }
      const pp = stepOrder(orderPos, order.length, repeat, -1);
      if (pp !== null) startPlayback(pp);
      else if (seekable) get().seek(0);
    },

    togglePause: () => {
      const { paused, playing } = get();
      if (!playing) return;
      if (paused) {
        set({ paused: false });
        void playerResume().catch(() => {});
      } else {
        set({ paused: true });
        void playerPause().catch(() => {});
      }
    },

    seek: (secs) => {
      if (!get().seekable) return;
      set({ positionSecs: secs });
      void playerSeek(secs).catch(() => {});
    },

    stop: async () => {
      userStopped = true;
      gaplessQueueRunning = false;
      set({
        ...idleState(),
        queue: [],
        queueIndex: -1,
        order: [],
        orderPos: -1,
      });
      await playerStop();
    },

    toggleShuffle: () => {
      const { shuffle, queue, order, orderPos } = get();
      const next = !shuffle;
      const curIdx = orderPos >= 0 ? order[orderPos] : -1;
      const start = curIdx != null && curIdx >= 0 ? curIdx : 0;
      const { order: newOrder, orderPos: newPos } = buildOrder(
        queue.length,
        next,
        start,
      );
      set({ shuffle: next, order: newOrder, orderPos: queue.length > 0 ? newPos : -1 });
      // Re-issue local gapless playback so upcoming tracks follow the new order.
      if (gaplessQueueRunning && queue.length > 0) startPlayback(newPos);
    },

    cycleRepeat: () => {
      const order: RepeatMode[] = ["off", "all", "one"];
      const next = order[(order.indexOf(get().repeat) + 1) % order.length]!;
      set({ repeat: next });
      // Repeat-one vs multi changes whether the engine owns the queue, so a
      // local track must be re-issued to switch modes cleanly.
      const { queue, orderPos } = get();
      const cur = orderPos >= 0 ? queue[get().order[orderPos]!] : undefined;
      if (cur?.source === "local" && get().playing) startPlayback(orderPos);
    },

    handleMediaCommand: (action, secs) => {
      const s = get();
      switch (action) {
        case "play":
          if (s.paused) s.togglePause();
          break;
        case "pause":
          if (s.playing && !s.paused) s.togglePause();
          break;
        case "toggle":
          s.togglePause();
          break;
        case "next":
          s.next();
          break;
        case "prev":
          s.prev();
          break;
        case "stop":
          void s.stop();
          break;
        case "seek":
          if (secs != null) s.seek(secs);
          break;
        case "seekForward":
          s.seek(s.positionSecs + (secs ?? 10));
          break;
        case "seekBackward":
          s.seek(Math.max(0, s.positionSecs - (secs ?? 10)));
          break;
      }
    },

    identifyNowPlaying: async () => {
      const s = get();
      const item = s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined;
      if (!item) {
        toast.info("Nothing is playing.");
        return;
      }
      // Fingerprinting needs the actual audio file; streams can't be matched.
      if (item.source !== "local" || !item.track?.path) {
        toast.info("Fingerprint ID needs a local file — fetching lyrics only.");
        return;
      }
      const path = item.track.path;
      let result;
      try {
        result = await identifyTrack(path);
      } catch {
        toast.error("Couldn't identify this track.");
        return;
      }
      if (!result) {
        toast.info("No match found for this track.");
        return;
      }
      if (!result.written) {
        toast.info("This track already has its info.");
        return;
      }
      // Reflect the recognized tags in the card, filling what was missing. A
      // title that's just the filename counts as missing.
      set((st) => {
        const prev = st.nowPlayingMeta;
        if (!prev) return {};
        const base = (path.split(/[/\\]/).pop() ?? "").replace(/\.[^.]+$/, "");
        const titleMissing = !prev.title?.trim() || prev.title === base;
        const title = titleMissing ? (result!.title ?? prev.title) : prev.title;
        return {
          nowPlayingMeta: {
            title,
            artist: prev.artist?.trim() ? prev.artist : result!.artist,
            album: prev.album?.trim() ? prev.album : result!.album,
            cover: prev.cover,
          },
          nowPlaying: title,
        };
      });
      const label = [result.title, result.artist].filter(Boolean).join(" — ");
      toast.success(label ? `Identified: ${label}` : "Track tags updated.");
    },

    applyChainPreset: async (id) => {
      await chainPresetApply(id);
      const st = await engineGetState();
      get().hydrate(st);
    },
  };
});
