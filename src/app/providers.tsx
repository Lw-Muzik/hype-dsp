import { useEffect } from "react";
import type { ReactNode } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  appInfo,
  engineGetState,
  onEngineFrame,
  onLinkNowPlaying,
  onMediaCommand,
  onNowPlaying,
  onOpenFiles,
  onProgress,
  onQueueIndex,
  onTransport,
  openFiles,
  takePendingOpenFiles,
} from "@/lib/ipc";
import { useUiStore } from "@/stores/ui";
import { useEngineStore, localItem } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";

/**
 * App-wide startup effects. Loads `AppInfo` and the engine state, then
 * subscribes to the real-time engine event stream (meter frames + transport).
 * Failures are non-fatal — the UI keeps its sensible defaults.
 */
export function Providers({ children }: { children: ReactNode }) {
  const setAppInfo = useUiStore((s) => s.setAppInfo);
  const hydrate = useEngineStore((s) => s.hydrate);
  const applyFrame = useEngineStore((s) => s.applyFrame);
  const applyProgress = useEngineStore((s) => s.applyProgress);
  const setPlaying = useEngineStore((s) => s.setPlaying);
  const castIncoming = useEngineStore((s) => s.castIncoming);
  const applyNowPlaying = useEngineStore((s) => s.applyNowPlaying);
  const applyQueueIndex = useEngineStore((s) => s.applyQueueIndex);
  const handleMediaCommand = useEngineStore((s) => s.handleMediaCommand);
  const playQueueItems = useEngineStore((s) => s.playQueueItems);
  const refreshLibrary = useLibraryStore((s) => s.refresh);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];

    // Files opened from the OS file manager: import them (so they persist under
    // Local), then play immediately — the first plays and the rest queue behind
    // it. Drained once for cold-launch/"Open With" before the UI mounted, and
    // again per warm `app:open_files` event while the app runs.
    const handleOpenFiles = async (paths: string[]) => {
      if (!paths.length) return;
      try {
        const tracks = await openFiles(paths);
        if (cancelled || !tracks.length) return;
        playQueueItems(tracks.map(localItem), 0);
        refreshLibrary();
      } catch {
        // Opening is best-effort; a failure shouldn't break startup.
      }
    };
    takePendingOpenFiles().then(handleOpenFiles).catch(() => {});
    onOpenFiles(handleOpenFiles)
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    appInfo()
      .then((info) => !cancelled && setAppInfo(info))
      .catch(() => {});

    engineGetState()
      .then((state) => !cancelled && hydrate(state))
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

    onLinkNowPlaying((np) => castIncoming(np.title, np.artist))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    onNowPlaying((meta) => applyNowPlaying(meta))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    onQueueIndex((index) => applyQueueIndex(index))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    onMediaCommand((c) => handleMediaCommand(c.action, c.secs))
      .then((un) => (cancelled ? un() : unlisteners.push(un)))
      .catch(() => {});

    return () => {
      cancelled = true;
      for (const un of unlisteners) un();
    };
  }, [
    setAppInfo,
    hydrate,
    applyFrame,
    applyProgress,
    setPlaying,
    castIncoming,
    applyNowPlaying,
    applyQueueIndex,
    handleMediaCommand,
    playQueueItems,
    refreshLibrary,
  ]);

  return <>{children}</>;
}
