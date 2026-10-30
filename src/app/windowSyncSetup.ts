import { appInfo, engineGetState } from "@/lib/ipc";
import {
  forwardToMain,
  requestSnapshot,
  serveMainInvocations,
  syncStore,
} from "@/lib/windowSync";
import { useThemeStore } from "@/stores/theme";
import { useEngineStore } from "@/stores/engine";
import { useSystemEqStore } from "@/stores/systemEq";
import { useVisualizerStore } from "@/stores/visualizer";
import { useAccountStore } from "@/stores/account";
import { useEntitlementStore } from "@/stores/entitlement";
import { useConnectStore } from "@/features/connect/store";
import { useUiStore } from "@/stores/ui";
import { useLibraryStore } from "@/stores/library";
import { useMusicLibraryStore } from "@/stores/musicLibrary";

/**
 * Which store state the main and Settings windows share: one `syncStore` per
 * store, each key mirrored both ways (see `@/lib/windowSync`). The spec is typed
 * against the store, so a misspelled key is a compile error.
 */
function syncAll(): Promise<(() => void)[]> {
  return Promise.all([
    syncStore(useThemeStore, "theme", { keys: ["choice", "blur", "resolved"] }),
    syncStore(useEngineStore, "engine", {
      keys: ["nowPlaying"],
      merge: { state: ["playback", "systemEqScope"] },
    }),
    syncStore(useSystemEqStore, "systemEq", { keys: ["enabled", "error"] }),
    syncStore(useVisualizerStore, "visualizer", {
      keys: ["available", "running", "settings", "current", "favorites", "autoChangePreset"],
    }),
    syncStore(useAccountStore, "account", { keys: ["status", "loading"] }),
    syncStore(useEntitlementStore, "entitlement", { keys: ["premium", "supporter", "loading"] }),
    syncStore(useConnectStore, "connect", { keys: ["status", "devices", "activeDeviceId"] }),
    syncStore(useUiStore, "ui", { keys: ["appInfo"] }),
  ]);
}

/**
 * Actions the Settings window cannot run itself: their effects live in the
 * main window's stores (the play queue and now-playing bar, the library views).
 */
export const FORWARDED_ACTIONS = [
  "engine.playPhoneList",
  "engine.playCloudList",
  "library.refresh",
  "musicLibrary.syncPhone",
  "musicLibrary.invalidateCloud",
  "musicLibrary.invalidateYtMusic",
] as const;

/** Main window: mirror the shared stores and serve forwarded actions. */
export function startMainWindowSync(): void {
  void syncAll().catch(() => {});
  void serveMainInvocations({
    "engine.playPhoneList": (...a: Parameters<ReturnType<typeof useEngineStore.getState>["playPhoneList"]>) =>
      useEngineStore.getState().playPhoneList(...a),
    "engine.playCloudList": (...a: Parameters<ReturnType<typeof useEngineStore.getState>["playCloudList"]>) =>
      useEngineStore.getState().playCloudList(...a),
    "library.refresh": () => useLibraryStore.getState().refresh(),
    "musicLibrary.syncPhone": () => useMusicLibraryStore.getState().syncPhone(),
    "musicLibrary.invalidateCloud": () => useMusicLibraryStore.getState().invalidateCloud(),
    "musicLibrary.invalidateYtMusic": () => useMusicLibraryStore.getState().invalidateYtMusic(),
  }).catch(() => {});
}

/**
 * Settings window: start from the main window's live state, route main-owned
 * actions to it, then mirror the shared stores both ways.
 */
export async function startSettingsWindowSync(): Promise<void> {
  // Hydrate first, before any sync is registered, so these values are not
  // broadcast to the main window as if the user had changed them.
  try {
    useUiStore.getState().setAppInfo(await appInfo());
  } catch {
    // Best-effort: About shows dashes until the sync snapshot arrives.
  }
  try {
    useEngineStore.getState().hydrate(await engineGetState());
  } catch {
    // Best-effort: the snapshot below carries the synced engine fields.
  }

  const engine = useEngineStore.getState();
  type EngineActions = typeof engine;
  useEngineStore.setState({
    playPhoneList: (...a: Parameters<EngineActions["playPhoneList"]>) =>
      forwardToMain("engine.playPhoneList", a),
    playCloudList: (...a: Parameters<EngineActions["playCloudList"]>) =>
      forwardToMain("engine.playCloudList", a),
  });
  useLibraryStore.setState({ refresh: () => forwardToMain("library.refresh", []) });
  useMusicLibraryStore.setState({
    syncPhone: () => forwardToMain("musicLibrary.syncPhone", []),
    invalidateCloud: () => forwardToMain("musicLibrary.invalidateCloud", []),
    invalidateYtMusic: () => forwardToMain("musicLibrary.invalidateYtMusic", []),
  });

  await syncAll();
  await requestSnapshot();
}
