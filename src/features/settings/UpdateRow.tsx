import { useEffect, useState } from "react";
import { CircleAlert, Download, RefreshCw } from "lucide-react";
import { Button } from "@/components/Button";
import {
  ipcErrorMessage,
  onUpdaterStatus,
  updaterCheckNow,
  updaterRestartNow,
  updaterStatus,
} from "@/lib/ipc";
import type { UpdaterStatus } from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { downloadPercent } from "@/features/update/updateNotice";

// Re-exported so existing imports (and its test) keep working.
export { downloadPercent };

/** A byte count at the only precision that means anything here. */
function mb(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/**
 * The update line in Settings → About.
 *
 * The unmissable notice now lives in the shell (`UpdateBanner` under the top bar
 * and `UpdatePill` in it, see `features/update`). This row stays as the
 * detailed view: it answers "am I up to date?", offers a manual check, and
 * shows download progress. Updates still install on quit either way.
 *
 * The backend owns all of it: checking, downloading and staging happen on a
 * cadence in Rust whether or not this component is mounted. This only renders
 * what it is told.
 */
export function UpdateRow() {
  const [status, setStatus] = useState<UpdaterStatus>({ state: "idle" });
  const [checking, setChecking] = useState(false);
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void updaterStatus().then((s) => !cancelled && setStatus(s));
    const unlisten = onUpdaterStatus((s) => !cancelled && setStatus(s));
    return () => {
      cancelled = true;
      void unlisten.then((f) => f());
    };
  }, []);

  const check = async () => {
    setChecking(true);
    try {
      await updaterCheckNow();
    } finally {
      setChecking(false);
    }
  };

  const restart = async () => {
    setRestarting(true);
    try {
      // Never resolves on success — the process is replaced mid-call.
      await updaterRestartNow();
    } catch (e) {
      setRestarting(false);
      toast.error(`Couldn't install the update: ${ipcErrorMessage(e)}`);
    }
  };

  if (status.state === "ready") {
    return (
      <div className="flex flex-col gap-2 rounded-control border border-accent/30 bg-accent-muted/40 px-3 py-2.5">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <p className="text-sm font-medium">Version {status.version} is ready</p>
            <p className="text-xs text-text-muted">
              It installs automatically when you quit HypeMuzik.
            </p>
          </div>
          <Button variant="primary" onClick={() => void restart()} disabled={restarting}>
            {restarting ? "Installing…" : "Restart now"}
          </Button>
        </div>
        {status.notes && (
          <p className="max-h-24 overflow-y-auto whitespace-pre-line text-xs leading-relaxed text-text-muted">
            {status.notes}
          </p>
        )}
      </div>
    );
  }

  if (status.state === "downloading") {
    const pct = downloadPercent(status.received, status.total);
    return (
      <div className="flex items-center gap-3 rounded-control border border-border bg-surface px-3 py-2.5">
        <Download className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="text-sm">Downloading {status.version}…</p>
          <div
            className="mt-1.5 h-1 overflow-hidden rounded-full bg-surface-overlay"
            role="progressbar"
            aria-label="Update download"
            aria-valuenow={pct ?? undefined}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div
              className={
                pct === null
                  ? "h-full w-1/3 animate-pulse rounded-full bg-accent"
                  : "h-full rounded-full bg-accent transition-[width] duration-300"
              }
              style={pct === null ? undefined : { width: `${pct}%` }}
            />
          </div>
        </div>
        <span className="shrink-0 text-xs tabular-nums text-text-faint">
          {pct === null ? mb(status.received) : `${pct}%`}
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between gap-3">
      <div className="min-w-0 text-sm">
        {status.state === "failed" ? (
          <p className="flex items-center gap-1.5 text-text-muted">
            <CircleAlert className="size-3.5 shrink-0 text-warning" aria-hidden="true" />
            Couldn&apos;t check for updates
          </p>
        ) : (
          <p className="text-text-muted">
            {status.state === "checking" ? "Checking for updates…" : "Up to date"}
          </p>
        )}
      </div>
      <Button
        variant="secondary"
        onClick={() => void check()}
        disabled={checking || status.state === "checking"}
      >
        <RefreshCw
          className={`size-4 ${checking || status.state === "checking" ? "animate-spin" : ""}`}
          aria-hidden="true"
        />
        Check for updates
      </Button>
    </div>
  );
}
