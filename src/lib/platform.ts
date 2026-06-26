/**
 * Platform helpers for window chrome.
 *
 * The window uses `titleBarStyle: "Overlay"` (see tauri.conf.json), which on
 * macOS draws the traffic-light buttons floating over our content's top-left
 * corner. We reserve a safe inset at the top of the chrome so nothing sits
 * under them — the desktop equivalent of a Flutter `SafeArea` top inset — and
 * make that strip draggable. Overlay is macOS-only; other platforms keep their
 * native title bar and need no inset.
 */
export const isMac =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/**
 * Height (px) of the traffic-light zone reserved at the top of the window
 * chrome on macOS. Matches the standard macOS title-bar height so the buttons
 * clear the content below. Zero elsewhere.
 */
export const TITLEBAR_INSET = isMac ? 28 : 0;
