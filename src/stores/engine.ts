import { create } from "zustand";
import {
  cloudPlay,
  engineSetBass,
  engineSetEq,
  engineSetMasterVolume,
  engineSetPower,
  engineSetRoom,
  engineSetSpatializer,
  engineSetSurround3d,
  ipcErrorMessage,
  linkPlay,
  engineSetPlayback,
  playerPause,
  playerPlayFile,
  playerPlayQueue,
  playerPlayRadio,
  playerResume,
  playerSeek,
  playerStop,
  profileClear,
} from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { BAND_COUNT } from "@/lib/types";
import type {
  CloudEntry,
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  LibraryTrack,
  MeterFrame,
  PhoneDevice,
  PhoneTrack,
  RadioStation,
  RoomState,
  SpatialMode,
  Surround3DState,
  TrackMeta,
  TransportProgress,
} from "@/lib/types";

/** How the engine reaches the audio for a queued item. */
export type QueueSource = "local" | "phone" | "cloud" | "radio" | "cast";

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
  // Exactly one payload is set, matching `source`:
  track?: LibraryTrack;
  device?: PhoneDevice;
  phoneTrack?: PhoneTrack;
  cloud?: CloudEntry;
  radioUrl?: string;
}

const defaultEngineState: EngineState = {
  power: true,
  masterVolume: 1,
  eq: { enabled: true, preGain: 0, bands: Array<number>(BAND_COUNT).fill(0) },
  bass: { enabled: false, amount: 0, harmonics: false },
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
  headphone: { enabled: false, preamp: 0, bands: [] },
  output: { gainDb: 0, limiterEnabled: true, ceilingDb: -0.3 },
  playback: { gapless: true, crossfadeSecs: 0 },
  activePresetId: null,
  activeProfileId: null,
};

const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

/* --------------------------------------------------------------- queue items */

function localItem(track: LibraryTrack): QueueItem {
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

function phoneItem(device: PhoneDevice, t: PhoneTrack): QueueItem {
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

function cloudItem(file: CloudEntry): QueueItem {
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

/** Now-playing card metadata derived from a queue item (before decode adds art). */
function itemMeta(item: QueueItem): TrackMeta {
  return { title: item.title, artist: item.artist, album: item.album, cover: null };
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
  metersLive: boolean;

  // transport
  playing: boolean;
  paused: boolean;
  nowPlaying: string | null;
  /** Rich now-playing metadata (tags + cover) for the docked bar. */
  nowPlayingMeta: TrackMeta | null;
  positionSecs: number;
  durationSecs: number | null;
  /** Whether the active source can be scrubbed (false for live radio). */
  seekable: boolean;

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
  setBass: (enabled: boolean, amount: number, harmonics: boolean) => void;
  setSpatializer: (enabled: boolean, amount: number, mode: SpatialMode) => void;
  setSurround3d: (next: Surround3DState) => void;
  setRoom: (next: RoomState) => void;
  applyProfile: (profile: HeadphoneProfile) => void;
  clearProfile: () => void;
  setPlayback: (gapless: boolean, crossfadeSecs: number) => void;

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
}

export const useEngineStore = create<EngineStore>((set, get) => {
  // Not part of rendered state: distinguishes user-stop from natural end.
  let userStopped = false;
  // Whether the engine is currently driving a multi-track gapless queue (true),
  // versus a single track / stream the store advances itself (false). Decides
  // how a natural end-of-stream is interpreted.
  let gaplessQueueRunning = false;

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
    positionSecs: 0,
    durationSecs: null,
    seekable: false,
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
      playing: true,
      paused: false,
      positionSecs: 0,
      durationSecs: item.durationSecs,
      // Optimistic; the backend's progress events correct this per source.
      seekable: item.source === "local",
    });

    const onError = (e: unknown) =>
      toast.error(`Couldn't play ${item.title}: ${ipcErrorMessage(e)}`);

    const { gapless, crossfadeSecs } = state.playback;
    const useEngineQueue =
      item.source === "local" && (gapless || crossfadeSecs > 0) && repeat !== "one";

    gaplessQueueRunning = useEngineQueue;

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
        void linkPlay(
          item.device!.id,
          item.phoneTrack!.id,
          item.phoneTrack!.ext,
          item.durationSecs,
        ).catch(onError);
        break;
      case "cloud":
        void cloudPlay(item.cloud!.provider, item.cloud!.id).catch(onError);
        break;
      case "radio":
        void playerPlayRadio(item.radioUrl!).catch(onError);
        break;
      case "cast":
        break; // already playing on the casting phone
    }
  };

  /** Replace the queue and start playing `index`. */
  const setQueueAndPlay = (items: QueueItem[], index: number) => {
    if (items.length === 0) return;
    const start = Math.max(0, Math.min(index, items.length - 1));
    const { order, orderPos } = buildOrder(items.length, get().shuffle, start);
    set({ queue: items, order, orderPos, queueIndex: order[orderPos]! });
    startPlayback(orderPos);
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
    metersLive: false,
    playing: false,
    paused: false,
    nowPlaying: null,
    nowPlayingMeta: null,
    positionSecs: 0,
    durationSecs: null,
    seekable: false,
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
    setBass: (enabled, amount, harmonics) => {
      set((s) => ({ state: { ...s.state, bass: { enabled, amount, harmonics } } }));
      void engineSetBass(enabled, amount, harmonics).catch(() => {});
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
      set((s) => ({ state: { ...s.state, playback: { gapless, crossfadeSecs } } }));
      void engineSetPlayback(gapless, crossfadeSecs).catch(() => {});
    },

    applyFrame: (frame) =>
      set({
        meters: frame.meters,
        ...(frame.spectrum ? { spectrum: frame.spectrum } : {}),
      }),

    applyProgress: (p) =>
      set((s) => ({
        positionSecs: p.positionSecs,
        // Keep a known (item-provided) duration until the backend learns one,
        // so streams show their length before the first progress with a value.
        durationSecs: p.durationSecs ?? s.durationSecs,
        paused: p.paused,
        seekable: p.seekable,
      })),

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
        return {
          nowPlayingMeta: {
            title: meta.title ?? prev?.title ?? s.nowPlaying,
            artist: meta.artist ?? prev?.artist ?? null,
            album: meta.album ?? prev?.album ?? null,
            cover: meta.cover ?? prev?.cover ?? null,
          },
          // Keep the title string in sync for views that match on it.
          ...(meta.title ? { nowPlaying: meta.title } : {}),
        };
      }),

    applyQueueIndex: (absPos) => {
      // Only the engine's gapless queue emits this; map its absolute index back
      // to our order position. Ignored for single-track / streamed playback.
      if (!gaplessQueueRunning) return;
      set((s) => {
        const qi = s.order[absPos];
        const item = qi != null ? s.queue[qi] : undefined;
        if (!item) return {};
        return {
          orderPos: absPos,
          queueIndex: qi!,
          nowPlaying: item.title,
          nowPlayingMeta: itemMeta(item),
          positionSecs: 0,
          durationSecs: item.durationSecs,
        };
      });
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
  };
});
