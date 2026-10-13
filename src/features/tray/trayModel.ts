import { EFFECTS, enabledOf } from "@/features/enhancer/effects/useEffectToggles";
import { visibleRoutes, type AppMode } from "@/features/mode/appMode";
import type { TrayModel } from "@/lib/ipc";
import type { EngineState, EqPreset } from "@/lib/types";
import type { UpdaterStatus } from "@/lib/ipc";

/** Plain values gathered from the stores; `buildTrayModel` is pure over these. */
export interface TrayInputs {
  nowPlaying: string | null;
  playing: boolean;
  paused: boolean;
  queueLength: number;
  queueIndex: number;
  mode: AppMode;
  modeSupported: boolean;
  engineState: EngineState;
  presets: Pick<EqPreset, "id" | "name">[];
  visualizerAvailable: boolean;
  visualizerRunning: boolean;
  updateStatus: UpdaterStatus;
}

/** Tray effects: every rack effect except Presets (it has no on/off state). */
export const TRAY_EFFECTS = EFFECTS.filter((e) => e.id !== "presets");

const trayLabel = (switchLabel: string): string => switchLabel.replace(/ on$/, "");

export function buildTrayModel(i: TrayInputs): TrayModel {
  return {
    nowPlaying: i.nowPlaying,
    playing: i.playing && !i.paused,
    canControl: i.nowPlaying !== null || (i.queueLength > 0 && i.queueIndex >= 0),
    mode: i.mode,
    modeSupported: i.modeSupported,
    power: i.engineState.power,
    effects: TRAY_EFFECTS.map((e) => {
      const on = enabledOf(e.id, i.engineState);
      // Headphone correction can only be turned on by picking a profile.
      return {
        id: e.id,
        label: trayLabel(e.switchLabel),
        on,
        enabled: e.id === "headphone" ? on : true,
      };
    }),
    presets: i.presets.map((p) => ({
      id: p.id,
      name: p.name,
      active: p.id === i.engineState.activePresetId,
    })),
    routes: visibleRoutes(i.mode).map((r) => ({ id: r.id, label: r.label })),
    visualizer: i.visualizerAvailable ? i.visualizerRunning : null,
    updateVersion: i.updateStatus.state === "ready" ? i.updateStatus.version : null,
  };
}

/** Deep equality for two models (all fields are JSON-shaped). */
export function sameTrayModel(a: TrayModel | null, b: TrayModel | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}
