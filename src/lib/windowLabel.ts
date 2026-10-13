import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/** The main window's label (`tauri.conf.json`). */
export const MAIN_WINDOW = "main";
/** The Settings child window's label (`src-tauri/src/commands/settings_window.rs`). */
export const SETTINGS_WINDOW = "settings";

/**
 * This webview's label. Outside Tauri (unit tests, a plain browser) there is no
 * window to ask, and that context behaves like the main window.
 */
export const WINDOW_LABEL: string = (() => {
  try {
    return getCurrentWebviewWindow().label;
  } catch {
    return MAIN_WINDOW;
  }
})();

export const isMainWindow = WINDOW_LABEL === MAIN_WINDOW;
export const isSettingsWindow = WINDOW_LABEL === SETTINGS_WINDOW;
