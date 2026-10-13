import type { UpdaterStatus } from "@/lib/ipc";

/**
 * Progress as a percentage, or `null` when the server sent no length.
 *
 * Exported because the distinction is the whole point: a chunked response has
 * no total, and rendering that as 0% gives a bar that sits still for the entire
 * download and looks stuck. `null` means "show indeterminate".
 */
export function downloadPercent(received: number, total: number | null): number | null {
  if (total === null || total <= 0) return null;
  return Math.min(100, Math.round((received / total) * 100));
}

/** What the shell should show for the updater, if anything. */
export type UpdateNotice =
  | { kind: "pill"; phase: "downloading"; version: string; percent: number | null }
  | { kind: "pill"; phase: "ready"; version: string; notes: string | null }
  | { kind: "banner"; version: string; notes: string | null };

/**
 * Pure mapping from updater status to a notice. A ready update is dismissed per
 * version: "Later" collapses the banner to a pill but never hides the update,
 * and a newer version brings the banner back. A failed check is deliberately
 * not surfaced here — it isn't the user's problem.
 */
export function noticeFor(
  status: UpdaterStatus,
  dismissedVersion: string | null,
): UpdateNotice | null {
  switch (status.state) {
    case "downloading":
      return {
        kind: "pill",
        phase: "downloading",
        version: status.version,
        percent: downloadPercent(status.received, status.total),
      };
    case "ready":
      return status.version === dismissedVersion
        ? { kind: "pill", phase: "ready", version: status.version, notes: status.notes }
        : { kind: "banner", version: status.version, notes: status.notes };
    default:
      return null;
  }
}
