import { create } from "zustand";
import {
  engineSetBass,
  engineSetEq,
  engineSetMasterVolume,
  engineSetPower,
  engineSetSpatializer,
  playerPlayFile,
  playerStop,
  profileClear,
} from "@/lib/ipc";
import { BAND_COUNT } from "@/lib/types";
import type {
  EngineFrame,
  EngineState,
  EqPreset,
  HeadphoneProfile,
  MeterFrame,
  SpatialMode,
} from "@/lib/types";

/**
 * Front-end mirror of the DSP engine state.
 *
 * `state` is hydrated from the backend and kept in sync: setters update
 * optimistically and dispatch a typed IPC command. `meters` and `spectrum` are
 * fed by the real `engine:frame` event (never synthesized); `metersLive` tracks
 * whether playback is active.
 */

const defaultEngineState: EngineState = {
  power: true,
  masterVolume: 1,
  eq: {
    enabled: true,
    preGain: 0,
    bands: Array<number>(BAND_COUNT).fill(0),
  },
  bass: { enabled: false, amount: 0, harmonics: false },
  spatializer: { enabled: false, amount: 0.5, mode: "crossfeed" },
  headphone: { enabled: false, preamp: 0, bands: [] },
  output: { gainDb: 0, limiterEnabled: true, ceilingDb: -0.3 },
  activePresetId: null,
  activeProfileId: null,
};

const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

interface EngineStore {
  state: EngineState;
  meters: MeterFrame;
  spectrum: number[];
  metersLive: boolean;
  playing: boolean;
  nowPlaying: string | null;

  hydrate: (state: EngineState) => void;

  setPower: (power: boolean) => void;
  setMasterVolume: (masterVolume: number) => void;

  /** Set a single EQ band (dB); clears the active preset (now custom). */
  setBand: (index: number, valueDb: number) => void;
  /** Replace all band gains at once (e.g. reset to flat); clears the preset. */
  setBands: (bands: number[]) => void;
  /** Set EQ pre-gain (dB). */
  setPreGain: (preGain: number) => void;
  /** Enable/disable the EQ stage. */
  setEqEnabled: (enabled: boolean) => void;
  /** Mirror an applied preset (the backend command applied it to the engine). */
  applyPreset: (preset: EqPreset) => void;

  setBass: (enabled: boolean, amount: number, harmonics: boolean) => void;
  setSpatializer: (
    enabled: boolean,
    amount: number,
    mode: SpatialMode,
  ) => void;
  /** Mirror an applied headphone profile (backend already applied it). */
  applyProfile: (profile: HeadphoneProfile) => void;
  clearProfile: () => void;

  applyFrame: (frame: EngineFrame) => void;
  setPlaying: (playing: boolean) => void;

  play: (path: string, name: string) => Promise<void>;
  stop: () => Promise<void>;
}

export const useEngineStore = create<EngineStore>((set, get) => {
  const pushEq = (eq: EngineState["eq"]) => {
    void engineSetEq(eq.bands, eq.preGain, eq.enabled).catch(() => {});
  };

  return {
    state: defaultEngineState,
    meters: idleMeters,
    spectrum: [],
    metersLive: false,
    playing: false,
    nowPlaying: null,

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
      const { eq } = get().state;
      const bands = eq.bands.slice();
      bands[index] = valueDb;
      const nextEq = { ...eq, bands };
      set((s) => ({
        state: { ...s.state, eq: nextEq, activePresetId: null },
      }));
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

    applyProfile: (profile) =>
      set((s) => ({
        state: {
          ...s.state,
          headphone: {
            enabled: true,
            preamp: profile.preamp,
            bands: profile.bands,
          },
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

    setPlaying: (playing) =>
      set((s) => ({
        playing,
        metersLive: playing,
        meters: playing ? s.meters : idleMeters,
        nowPlaying: playing ? s.nowPlaying : null,
      })),

    play: async (path, name) => {
      await playerPlayFile(path);
      set({ nowPlaying: name, playing: true, metersLive: true });
    },

    stop: async () => {
      await playerStop();
      set({ playing: false, metersLive: false, meters: idleMeters, nowPlaying: null });
    },
  };
});
