import { useEffect } from "react";
import type { ReactNode } from "react";
import { appInfo } from "@/lib/ipc";
import { useUiStore } from "@/stores/ui";

/**
 * App-wide providers and startup effects. Loads `AppInfo` once on mount; a
 * failure here is non-fatal (the UI keeps its sensible defaults). In Phase 2
 * this is where the engine `EngineFrame` channel subscription will live.
 */
export function Providers({ children }: { children: ReactNode }) {
  const setAppInfo = useUiStore((s) => s.setAppInfo);

  useEffect(() => {
    let cancelled = false;
    appInfo()
      .then((info) => {
        if (!cancelled) setAppInfo(info);
      })
      .catch(() => {
        /* non-fatal: keep defaults */
      });
    return () => {
      cancelled = true;
    };
  }, [setAppInfo]);

  return <>{children}</>;
}
