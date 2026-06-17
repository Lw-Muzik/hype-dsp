import { create } from "zustand";
import {
  engineSetMasterVolume,
  engineSetPower,
  playerPlayFile,
  playerStop,
} from "@/lib/ipc";
import { BAND_COUNT } from "@/lib/types";
import type { EngineState, MeterFrame } from "@/lib/types";

/**
 * Front-end mirror of the DSP engine state.
 *
 * `state` is hydrated from the backend and kept in sync: setters update
 * optimistically and dispatch a typed IPC command. `meters` is fed by the real
 * `engine:frame` event (never synthesized); `metersLive` tracks whether
 * playback is active.
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
  output: { gainDb: 0, limiterEnabled: true, ceilingDb: -0.3 },
  activePresetId: null,
  activeProfileId: null,
};

const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

interface EngineStore {
  state: EngineState;
  meters: MeterFrame;
  metersLive: boolean;
  playing: boolean;
  nowPlaying: string | null;

  /** Replace the mirrored state (used on startup hydration). */
  hydrate: (state: EngineState) => void;

  setPower: (power: boolean) => void;
  setMasterVolume: (masterVolume: number) => void;

  /** Apply a real meter frame from the engine. */
  applyMeterFrame: (meters: MeterFrame) => void;
  /** React to a play/stop transition from the engine. */
  setPlaying: (playing: boolean) => void;

  /** Decode + play a file; resolves on success, throws the IPC error. */
  play: (path: string, name: string) => Promise<void>;
  stop: () => Promise<void>;
}

export const useEngineStore = create<EngineStore>((set) => ({
  state: defaultEngineState,
  meters: idleMeters,
  metersLive: false,
  playing: false,
  nowPlaying: null,

  hydrate: (state) => set({ state }),

  setPower: (power) => {
    set((store) => ({ state: { ...store.state, power } }));
    void engineSetPower(power).catch(() => {});
  },

  setMasterVolume: (masterVolume) => {
    set((store) => ({ state: { ...store.state, masterVolume } }));
    void engineSetMasterVolume(masterVolume).catch(() => {});
  },

  applyMeterFrame: (meters) => set({ meters }),

  setPlaying: (playing) =>
    set((store) => ({
      playing,
      metersLive: playing,
      meters: playing ? store.meters : idleMeters,
      nowPlaying: playing ? store.nowPlaying : null,
    })),

  play: async (path, name) => {
    await playerPlayFile(path);
    set({ nowPlaying: name, playing: true, metersLive: true });
  },

  stop: async () => {
    await playerStop();
    set({ playing: false, metersLive: false, meters: idleMeters, nowPlaying: null });
  },
}));
