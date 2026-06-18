import { create } from "zustand";
import {
  engineSetBass,
  engineSetEq,
  engineSetMasterVolume,
  engineSetPower,
  engineSetRoom,
  engineSetSpatializer,
  engineSetSurround3d,
  ipcErrorMessage,
  playerPause,
  playerPlayFile,
  playerPlayRadio,
  playerResume,
  playerSeek,
  playerStop,
  profileClear,
} from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { BAND_COUNT } from "@/lib/types";
import type {
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  LibraryTrack,
  MeterFrame,
  RadioStation,
  RoomState,
  SpatialMode,
  Surround3DState,
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
  activePresetId: null,
  activeProfileId: null,
};

const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

/** Synthesize a minimal track for an ad-hoc opened file. */
function fileTrack(path: string, title: string): LibraryTrack {
  return { path, title, artist: null, album: null, durationSecs: null };
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

  applyFrame: (frame: EngineFrame) => void;
  applyProgress: (p: TransportProgress) => void;
  setPlaying: (playing: boolean) => void;

  /** Play an ad-hoc file (single-item queue). Throws on IPC error. */
  play: (path: string, name: string) => Promise<void>;
  /** Play a track list starting at `index`. */
  playFromList: (tracks: LibraryTrack[], index: number) => void;
  /** Stream an internet radio station (live; no queue/duration). */
  playRadio: (station: RadioStation) => void;
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
      playing: true,
      paused: false,
      positionSecs: 0,
      durationSecs: track.durationSecs,
    });
  };

  return {
    state: defaultEngineState,
    meters: idleMeters,
    spectrum: [],
    metersLive: false,
    playing: false,
    paused: false,
    nowPlaying: null,
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
        positionSecs: 0,
      });
    },

    play: async (path, name) => {
      set({ queue: [fileTrack(path, name)], queueIndex: 0 });
      await startTrack(fileTrack(path, name));
    },

    playFromList: (tracks, index) => {
      const track = tracks[index];
      if (!track) return;
      set({ queue: tracks, queueIndex: index });
      void startTrack(track).catch((e) =>
        toast.error(`Couldn't play ${track.title}: ${ipcErrorMessage(e)}`),
      );
    },

    playRadio: (station) => {
      set({
        nowPlaying: station.name,
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

    next: () => {
      const { queue, queueIndex } = get();
      const track = queue[queueIndex + 1];
      if (!track) return;
      set({ queueIndex: queueIndex + 1 });
      void startTrack(track).catch((e) =>
        toast.error(`Couldn't play ${track.title}: ${ipcErrorMessage(e)}`),
      );
    },

    prev: () => {
      const { queue, queueIndex } = get();
      const track = queue[queueIndex - 1];
      if (!track) return;
      set({ queueIndex: queueIndex - 1 });
      void startTrack(track).catch((e) =>
        toast.error(`Couldn't play ${track.title}: ${ipcErrorMessage(e)}`),
      );
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
        positionSecs: 0,
        queue: [],
        queueIndex: -1,
      });
      await playerStop();
    },
  };
});
