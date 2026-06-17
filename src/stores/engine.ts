import { create } from "zustand";
import { BAND_COUNT } from "@/lib/types";
import type { EngineState, MeterFrame } from "@/lib/types";

/**
 * Front-end mirror of the DSP engine state.
 *
 * Phase 0: `power` and `masterVolume` are local UI state with no backend
 * behind them yet, and meters are inert (zeroed) — never synthesized. In
 * Phase 2 the setters become typed IPC commands and `meters` is fed by the
 * real `EngineFrame` channel from the audio thread.
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

/** Idle meters — zeros until the real engine emits frames (Phase 2). */
const idleMeters: MeterFrame = { peak: [0, 0], rms: [0, 0] };

interface EngineStore {
  state: EngineState;
  meters: MeterFrame;
  /** Whether real meter frames are flowing yet (false until Phase 2). */
  metersLive: boolean;

  setPower: (power: boolean) => void;
  setMasterVolume: (masterVolume: number) => void;
}

export const useEngineStore = create<EngineStore>((set) => ({
  state: defaultEngineState,
  meters: idleMeters,
  metersLive: false,

  setPower: (power) =>
    set((store) => ({ state: { ...store.state, power } })),
  setMasterVolume: (masterVolume) =>
    set((store) => ({ state: { ...store.state, masterVolume } })),
}));
