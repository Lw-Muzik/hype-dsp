import { EFFECTS, enabledOf, toggleEffect, type EffectId } from "@/features/enhancer/effects/useEffectToggles";
import { modeFromSystemEq, visibleRoutes, type AppMode } from "@/features/mode/appMode";
import { switchMode } from "@/features/mode/switchMode";
import { eqApplyPreset } from "@/lib/ipc";
import { useEngineStore } from "@/stores/engine";
import { useSystemEqStore } from "@/stores/systemEq";
import { useUiStore, type Route } from "@/stores/ui";
import { useUpdateStore } from "@/stores/update";
import { useVisualizerStore } from "@/stores/visualizer";

export interface TrayActionDeps {
  mediaCommand: (cmd: "toggle" | "next" | "prev") => void;
  switchMode: (target: AppMode) => void | Promise<unknown>;
  getMode: () => AppMode;
  getPower: () => boolean;
  setPower: (on: boolean) => void;
  isEffectOn: (id: EffectId) => boolean;
  toggleEffect: (id: EffectId, on: boolean) => void;
  applyPreset: (id: string) => void | Promise<unknown>;
  setRoute: (route: Route) => void;
  isRouteVisible: (route: string, mode: AppMode) => boolean;
  toggleVisualizer: () => void;
  restartUpdate: () => void | Promise<unknown>;
}

export const defaultTrayDeps: TrayActionDeps = {
  mediaCommand: (cmd) => useEngineStore.getState().handleMediaCommand(cmd, null),
  switchMode: (t) => switchMode(t),
  getMode: () => modeFromSystemEq(useSystemEqStore.getState().enabled),
  getPower: () => useEngineStore.getState().state.power,
  setPower: (on) => useEngineStore.getState().setPower(on),
  isEffectOn: (id) => enabledOf(id, useEngineStore.getState().state),
  toggleEffect,
  applyPreset: async (id) => {
    const preset = await eqApplyPreset(id);
    useEngineStore.getState().applyPreset(preset);
  },
  setRoute: (r) => useUiStore.getState().setRoute(r),
  isRouteVisible: (route, mode) => visibleRoutes(mode).some((r) => r.id === route),
  toggleVisualizer: () => useVisualizerStore.getState().toggle(),
  restartUpdate: () => useUpdateStore.getState().restart(),
};

const TOGGLEABLE = new Set<string>(EFFECTS.filter((e) => e.id !== "presets").map((e) => e.id));

/** Run one tray menu action through the same store actions the UI uses.
 *  Unknown ids are ignored; rejections are swallowed (the caller re-syncs). */
export async function runTrayAction(
  action: string,
  deps: TrayActionDeps = defaultTrayDeps,
): Promise<void> {
  try {
    switch (action) {
      case "play_pause":
        return deps.mediaCommand("toggle");
      case "next":
        return deps.mediaCommand("next");
      case "prev":
        return deps.mediaCommand("prev");
      // Picking the current mode is a no-op (re-enabling would rebuild the
      // system-audio tap for nothing); the re-sync restores the checkmark.
      case "mode:player":
        if (deps.getMode() !== "player") await deps.switchMode("player");
        return;
      case "mode:equalizer":
        if (deps.getMode() !== "equalizer") await deps.switchMode("equalizer");
        return;
      case "power":
        return deps.setPower(!deps.getPower());
      case "presets:open":
        return deps.setRoute("equalizer");
      case "visualizer":
        return deps.toggleVisualizer();
      case "update_restart":
        await deps.restartUpdate();
        return;
    }
    if (action.startsWith("fx:")) {
      const id = action.slice(3);
      if (!TOGGLEABLE.has(id)) return;
      return deps.toggleEffect(id as EffectId, !deps.isEffectOn(id as EffectId));
    }
    if (action.startsWith("preset:")) {
      const id = action.slice(7);
      if (id) await deps.applyPreset(id);
      return;
    }
    if (action.startsWith("go:")) {
      const route = action.slice(3);
      if (deps.isRouteVisible(route, deps.getMode())) deps.setRoute(route as Route);
    }
  } catch {
    // A refused/failed action must not break the tray; the caller re-syncs.
  }
}
