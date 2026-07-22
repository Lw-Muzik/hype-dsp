/** Turning `ytmusic-setup-progress` events into the card's human line —
 *  pure so the wording and math are testable without a browser. */

/** Payload of the `ytmusic-setup-progress` Tauri event. */
export interface SetupProgressEvent {
  /** `yt-dlp` or `ffmpeg`. */
  tool: string;
  /** `downloading` | `verifying` | `installing`. */
  phase: string;
  received: number;
  total: number | null;
}

/** Whole megabytes, floored — progress copy, not accounting. */
function mb(bytes: number): number {
  return Math.floor(bytes / (1024 * 1024));
}

/** One line describing where setup is, e.g. "Downloading yt-dlp… 12 of 17 MB". */
export function setupStatusLine(e: SetupProgressEvent): string {
  switch (e.phase) {
    case "downloading":
      return e.total
        ? `Downloading ${e.tool}… ${mb(e.received)} of ${mb(e.total)} MB`
        : `Downloading ${e.tool}… ${mb(e.received)} MB`;
    case "verifying":
      return `Verifying ${e.tool}…`;
    case "installing":
      return `Installing ${e.tool}…`;
    default:
      return `Setting up ${e.tool}…`;
  }
}

/** 0–100 for the bar, or null while indeterminate (no total, or a phase
 *  without byte counts). */
export function setupPercent(e: SetupProgressEvent): number | null {
  if (e.phase !== "downloading" || !e.total) return null;
  return Math.max(0, Math.min(100, Math.round((e.received / e.total) * 100)));
}
