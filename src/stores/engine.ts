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

/** Synthesize a minimal track for an ad-hoc opened file. */
function fileTrack(path: string, title: string): LibraryTrack {
  return { path, title, artist: null, album: null, durationSecs: null };
}

/** An initial now-playing card from what we know before decode fills in tags. */
function initialMeta(
  title: string,
  artist: string | null = null,
  album: string | null = null,
): TrackMeta {
  return { title, artist, album, cover: null };
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
  queue: LibraryTrack[];
  queueIndex: number;

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
  /** Play a track list starting at `index`. */
  playFromList: (tracks: LibraryTrack[], index: number) => void;
  /** Stream an internet radio station (live; no queue/duration). */
  playRadio: (station: RadioStation) => void;
  /** Stream a cloud file (Drive/Dropbox) through the chain. */
  playCloud: (file: CloudEntry) => void;
  /** Stream a track from a paired phone through the chain. */
  playPhone: (device: PhoneDevice, track: PhoneTrack) => void;
  /** Reflect a track a phone has cast to this desktop (started server-side). */
  castIncoming: (title: string, artist: string | null) => void;
  next: () => void;
  prev: () => void;
  togglePause: () => void;
  seek: (secs: number) => void;
  stop: () => Promise<void>;
}

export const useEngineStore = create<EngineStore>((set, get) => {
  // Not part of rendered state: distinguishes user-stop from natural end.
  let userStopped = false;

  const pushEq = (eq: EngineState["eq"]) => {
    void engineSetEq(eq.bands, eq.preGain, eq.enabled).catch(() => {});
  };

  const startTrack = async (track: LibraryTrack) => {
    await playerPlayFile(track.path);
    set({
      nowPlaying: track.title,
      nowPlayingMeta: initialMeta(track.title, track.artist, track.album),
      playing: true,
      paused: false,
      positionSecs: 0,
      durationSecs: track.durationSecs,
    });
  };

  /** Play `tracks[index]`, gaplessly (engine owns the queue) when enabled, else
   *  one track at a time (the store advances on each track end). */
  const playFrom = (tracks: LibraryTrack[], index: number) => {
    const track = tracks[index];
    if (!track) return;
    set({ queue: tracks, queueIndex: index });
    const { gapless, crossfadeSecs } = get().state.playback;
    const onError = (e: unknown) =>
      toast.error(`Couldn't play ${track.title}: ${ipcErrorMessage(e)}`);

    if (gapless || crossfadeSecs > 0) {
      set({
        nowPlaying: track.title,
        nowPlayingMeta: initialMeta(track.title, track.artist, track.album),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: track.durationSecs,
      });
      void playerPlayQueue(
        tracks.map((t) => t.path),
        index,
      ).catch(onError);
    } else {
      void startTrack(track).catch(onError);
    }
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
    queue: [],
    queueIndex: -1,

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
      set({
        positionSecs: p.positionSecs,
        durationSecs: p.durationSecs,
        paused: p.paused,
      }),

    setPlaying: (playing) => {
      if (playing) {
        set({ playing: true, metersLive: true });
        return;
      }
      // Stopped/ended.
      if (userStopped) {
        userStopped = false;
        set({
          playing: false,
          metersLive: false,
          meters: idleMeters,
          nowPlaying: null,
          nowPlayingMeta: null,
          positionSecs: 0,
        });
        return;
      }
      const { queue, queueIndex } = get();
      if (queueIndex >= 0 && queueIndex + 1 < queue.length) {
        get().next();
        return;
      }
      set({
        playing: false,
        metersLive: false,
        meters: idleMeters,
        nowPlaying: null,
        nowPlayingMeta: null,
        positionSecs: 0,
      });
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

    applyQueueIndex: (index) =>
      set((s) => {
        const track = s.queue[index];
        if (!track) return {};
        // Reset the card for the new track; the now_playing event adds the cover.
        return {
          queueIndex: index,
          nowPlaying: track.title,
          nowPlayingMeta: initialMeta(track.title, track.artist, track.album),
          positionSecs: 0,
        };
      }),

    play: async (path, name) => {
      set({ queue: [fileTrack(path, name)], queueIndex: 0 });
      await startTrack(fileTrack(path, name));
    },

    playFromList: (tracks, index) => playFrom(tracks, index),

    playRadio: (station) => {
      set({
        nowPlaying: station.name,
        nowPlayingMeta: initialMeta(station.name),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: null,
        queue: [],
        queueIndex: -1,
      });
      void playerPlayRadio(station.url).catch((e) =>
        toast.error(`Couldn't stream ${station.name}: ${ipcErrorMessage(e)}`),
      );
    },

    playCloud: (file) => {
      set({
        nowPlaying: file.name,
        nowPlayingMeta: initialMeta(file.name),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: null,
        queue: [],
        queueIndex: -1,
      });
      void cloudPlay(file.provider, file.id).catch((e) =>
        toast.error(`Couldn't play ${file.name}: ${ipcErrorMessage(e)}`),
      );
    },

    playPhone: (device, track) => {
      set({
        nowPlaying: track.title,
        nowPlayingMeta: initialMeta(track.title, track.artist, track.album),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: null,
        queue: [],
        queueIndex: -1,
      });
      void linkPlay(device.id, track.id, track.ext).catch((e) =>
        toast.error(`Couldn't play ${track.title}: ${ipcErrorMessage(e)}`),
      );
    },

    castIncoming: (title, artist) => {
      set({
        nowPlaying: title,
        nowPlayingMeta: initialMeta(title, artist),
        playing: true,
        paused: false,
        positionSecs: 0,
        durationSecs: null,
        queue: [],
        queueIndex: -1,
      });
    },

    next: () => {
      const { queue, queueIndex } = get();
      if (queueIndex + 1 < queue.length) playFrom(queue, queueIndex + 1);
    },

    prev: () => {
      const { queue, queueIndex } = get();
      if (queueIndex - 1 >= 0) playFrom(queue, queueIndex - 1);
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
      set({ positionSecs: secs });
      void playerSeek(secs).catch(() => {});
    },

    stop: async () => {
      userStopped = true;
      set({
        playing: false,
        paused: false,
        metersLive: false,
        meters: idleMeters,
        nowPlaying: null,
        nowPlayingMeta: null,
        positionSecs: 0,
        queue: [],
        queueIndex: -1,
      });
      await playerStop();
    },
  };
});
