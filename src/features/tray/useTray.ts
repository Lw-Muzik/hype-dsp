import { useEffect } from "react";
import { modeFromSystemEq } from "@/features/mode/appMode";
import { onTrayAction, systemAudioStatus, trayUpdate, eqListPresets } from "@/lib/ipc";
import type { TrayModel } from "@/lib/ipc";
import type { EqPreset } from "@/lib/types";
import { useEngineStore } from "@/stores/engine";
import { useSystemEqStore } from "@/stores/systemEq";
import { useUpdateStore } from "@/stores/update";
import { useVisualizerStore } from "@/stores/visualizer";
import { runTrayAction } from "./trayActions";
import { buildTrayModel, sameTrayModel } from "./trayModel";

export const TRAY_DEBOUNCE_MS = 120;

/**
 * Mirror app state into the native tray menu and execute its clicks. Mount once,
 * in the main window only.
 */
export function useTray(): void {
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let last: TrayModel | null = null;
    let presets: Pick<EqPreset, "id" | "name">[] = [];
    let modeSupported = true;
    let unlisten: (() => void) | undefined;

    const build = (): TrayModel => {
      const e = useEngineStore.getState();
      const v = useVisualizerStore.getState();
      return buildTrayModel({
        nowPlaying: e.nowPlaying,
        playing: e.playing,
        paused: e.paused,
        queueLength: e.queue.length,
        queueIndex: e.queueIndex,
        mode: modeFromSystemEq(useSystemEqStore.getState().enabled),
        modeSupported,
        engineState: e.state,
        presets,
        visualizerAvailable: v.available,
        visualizerRunning: v.running,
        updateStatus: useUpdateStore.getState().status,
      });
    };

    const push = (force: boolean) => {
      if (disposed) return;
      if (timer !== undefined) {
        clearTimeout(timer);
        timer = undefined;
      }
      try {
        const model = build();
        if (!force && sameTrayModel(model, last)) return;
        last = model;
        void trayUpdate(model).catch(() => {});
      } catch {
        // No tray backend / half-initialised stores — never break the app.
      }
    };

    // Never resets a pending push: the engine store changes ~30 times a second
    // while audio plays (meters), and a resetting debounce would never fire.
    const schedule = () => {
      if (disposed || timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        push(false);
      }, TRAY_DEBOUNCE_MS);
    };

    const loadPresets = () => {
      void eqListPresets()
        .then((list) => {
          if (disposed) return;
          presets = list.map((p) => ({ id: p.id, name: p.name }));
          schedule();
        })
        .catch(() => {});
    };

    const unsubs = [
      useEngineStore.subscribe((s, prev) => {
        if (s.state.activePresetId !== prev.state.activePresetId) loadPresets();
        // Only what the menu shows; meter and progress frames are ignored.
        if (
          s.state !== prev.state ||
          s.nowPlaying !== prev.nowPlaying ||
          s.playing !== prev.playing ||
          s.paused !== prev.paused ||
          s.queue !== prev.queue ||
          s.queueIndex !== prev.queueIndex
        ) {
          schedule();
        }
      }),
      useSystemEqStore.subscribe(schedule),
      useVisualizerStore.subscribe(schedule),
      useUpdateStore.subscribe(schedule),
    ];

    const onFocus = () => loadPresets();
    window.addEventListener("focus", onFocus);

    useVisualizerStore.getState().probe();
    loadPresets();
    void systemAudioStatus()
      .then((s) => {
        modeSupported = s.supported;
        schedule();
      })
      .catch(() => {});
    schedule();

    void onTrayAction((action) => {
      void runTrayAction(action).finally(() => push(true));
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
      unsubs.forEach((u) => u());
      window.removeEventListener("focus", onFocus);
      unlisten?.();
    };
  }, []);
}
