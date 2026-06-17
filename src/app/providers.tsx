import { useEffect } from "react";
import type { ReactNode } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  appInfo,
  engineGetState,
  licenseStatus,
  onEngineFrame,
  onProgress,
  onTransport,
} from "@/lib/ipc";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";

/**
 * App-wide startup effects. Loads `AppInfo` and the engine state, then
 * subscribes to the real-time engine event stream (meter frames + transport).
 * Failures are non-fatal — the UI keeps its sensible defaults.
 */
export function Providers({ children }: { children: ReactNode }) {
  const setAppInfo = useUiStore((s) => s.setAppInfo);
  const setLicense = useUiStore((s) => s.setLicense);
  const hydrate = useEngineStore((s) => s.hydrate);
  const applyFrame = useEngineStore((s) => s.applyFrame);
  const applyProgress = useEngineStore((s) => s.applyProgress);
  const setPlaying = useEngineStore((s) => s.setPlaying);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];

    appInfo()
      .then((info) => !cancelled && setAppInfo(info))
      .catch(() => {});

    engineGetState()
      .then((state) => !cancelled && hydrate(state))
      .catch(() => {});

    licenseStatus()
      .then((status) => !cancelled && setLicense(status))
      .catch(() => {});

    onEngineFrame((frame) => applyFrame(frame))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    onTransport((playing) => setPlaying(playing))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    onProgress((p) => applyProgress(p))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    return () => {
      cancelled = true;
      for (const un of unlisteners) un();
    };
  }, [setAppInfo, setLicense, hydrate, applyFrame, applyProgress, setPlaying]);

  return <>{children}</>;
}
