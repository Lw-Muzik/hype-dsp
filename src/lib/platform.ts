/**
 * Platform helpers — window chrome, and the few places copy must name the OS.
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

export const isWindows =
  typeof navigator !== "undefined" && /Win/i.test(navigator.userAgent);

/** Which OS we're on, for copy that differs per platform (setup instructions,
 *  what to call this machine). Everything that isn't macOS or Windows is
 *  treated as Linux — the third target this app ships for. */
export type HostOs = "mac" | "windows" | "linux";
export const HOST_OS: HostOs = isMac ? "mac" : isWindows ? "windows" : "linux";

/** What to call this machine in UI copy. */
export const THIS_COMPUTER =
  HOST_OS === "mac" ? "this Mac" : HOST_OS === "windows" ? "this PC" : "this computer";

/**
 * Height (px) of the traffic-light zone reserved at the top of the window
 * chrome on macOS. Matches the standard macOS title-bar height so the buttons
 * clear the content below. Zero elsewhere.
 */
export const TITLEBAR_INSET = isMac ? 28 : 0;
