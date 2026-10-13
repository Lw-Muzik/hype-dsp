import { useCallback, useEffect, useRef, useState } from "react";
import { AudioWaveform, Users } from "lucide-react";
import {
  mixerListSessions,
  systemEqStatus,
  type SystemEqRuntimeStatus,
} from "@/lib/ipc";
import type { AppSession, SystemEqScopeMode } from "@/lib/types";
import { useEngineStore } from "@/stores/engine";
import { useSystemEqStore } from "@/stores/systemEq";
import { useUiStore } from "@/stores/ui";
import { Button } from "@/components/Button";
import { cn } from "@/lib/cn";
import { isMac } from "@/lib/platform";
import { scopeSummary, setScopeMode, toggleScopeApp } from "./systemEqScope";

const STATUS_LABEL: Record<SystemEqRuntimeStatus, string> = {
  active: "Equalizing system audio",
  recovering: "Recovering…",
  disabled: "Stopped",
};

const DOT: Record<SystemEqRuntimeStatus, string> = {
  active: "bg-success",
  recovering: "bg-accent motion-safe:animate-pulse",
  disabled: "bg-text-faint",
};

const SCOPE_MODES: [SystemEqScopeMode, string][] = [
  ["all", "All apps"],
  ["only", "Only selected"],
  ["except", "All except"],
];

/** Live system-EQ status, polled while mounted (same cadence as Settings). */
function useRuntimeStatus(): SystemEqRuntimeStatus {
  const [status, setStatus] = useState<SystemEqRuntimeStatus>("disabled");
  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      systemEqStatus()
        .then((s) => !cancelled && setStatus(s))
        .catch(() => {});
    };
    poll();
    const id = window.setInterval(poll, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);
  return status;
}

/** Equalizer mode's replacement for the sidebar now-playing card. */
export function SystemAudioPanel({ collapsed }: { collapsed: boolean }) {
  const status = useRuntimeStatus();
  const enable = useSystemEqStore((s) => s.enable);
  const storeError = useSystemEqStore((s) => s.error);
  const scope = useEngineStore((s) => s.state.systemEqScope);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);
  const [open, setOpen] = useState(false);
  const [apps, setApps] = useState<AppSession[]>([]);
  const [error, setError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const refreshApps = useCallback(() => {
    mixerListSessions()
      .then((snap) => setApps(snap.sessions))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const run = async (p: Promise<string | null>) => {
    setError(null);
    const message = await p;
    if (message) setError(message);
  };

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={toggleSidebar}
        aria-label={`${STATUS_LABEL[status]}. Expand sidebar`}
        title={STATUS_LABEL[status]}
        className="relative mx-auto mb-2 grid size-11 place-items-center rounded-lg bg-surface-overlay text-text-muted transition-colors hover:text-text"
      >
        <AudioWaveform className="size-5" aria-hidden="true" />
        <span
          aria-hidden="true"
          className={cn("absolute right-1.5 top-1.5 size-2 rounded-full", DOT[status])}
        />
      </button>
    );
  }

  const shownError = error ?? storeError;

  return (
    <div ref={rootRef} className="relative mx-3 mb-2">
      <div className="rounded-card border border-border bg-surface p-3">
        <div className="flex items-center gap-2">
          <span
            aria-hidden="true"
            className={cn("size-2 shrink-0 rounded-full", DOT[status])}
          />
          <p className="min-w-0 flex-1 truncate text-sm font-medium" aria-live="polite">
            {STATUS_LABEL[status]}
          </p>
        </div>
        <p className="mt-1 text-xs text-text-muted">{scopeSummary(scope)}</p>
        {shownError && <p className="mt-1 text-xs text-danger">{shownError}</p>}
        <div className="mt-2.5 flex flex-wrap gap-2">
          {status === "disabled" && (
            <Button variant="primary" onClick={() => void enable()}>
              Restart
            </Button>
          )}
          {isMac && (
            <Button
              variant="secondary"
              onClick={() => {
                refreshApps();
                setOpen((o) => !o);
              }}
              aria-expanded={open}
              aria-haspopup="dialog"
            >
              <Users className="size-4" aria-hidden="true" /> Choose apps…
            </Button>
          )}
        </div>
      </div>

      {open && isMac && (
        <div
          role="dialog"
          aria-label="Choose apps to equalize"
          className="absolute bottom-full left-0 z-30 mb-2 w-72 rounded-card border border-border bg-surface-raised p-3 shadow-lg"
        >
          <div className="flex gap-1 rounded-control bg-surface-overlay p-0.5">
            {SCOPE_MODES.map(([mode, label]) => (
              <button
                key={mode}
                type="button"
                onClick={() => {
                  refreshApps();
                  void run(setScopeMode(mode));
                }}
                aria-pressed={scope.mode === mode}
                className={cn(
                  "flex-1 rounded-[6px] px-2 py-1 text-xs transition-colors",
                  scope.mode === mode
                    ? "bg-accent-muted text-accent-strong"
                    : "text-text-muted hover:text-text",
                )}
              >
                {label}
              </button>
            ))}
          </div>

          {scope.mode !== "all" && (
            <div className="mt-2.5 max-h-48 overflow-y-auto rounded-md border border-border">
              {apps.length === 0 ? (
                <p className="px-3 py-3 text-xs text-text-faint">
                  No apps are playing audio right now. Start playback in another app,
                  then reopen this list.
                </p>
              ) : (
                <ul className="divide-y divide-border/60">
                  {apps.map((app) => {
                    const checked = scope.apps.includes(app.id);
                    return (
                      <li key={app.id}>
                        <button
                          type="button"
                          onClick={() => void run(toggleScopeApp(app.id))}
                          aria-pressed={checked}
                          className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface-overlay"
                        >
                          <span
                            aria-hidden="true"
                            className={cn(
                              "flex size-4 shrink-0 items-center justify-center rounded-[4px] border text-[10px]",
                              checked
                                ? "border-accent bg-accent-muted text-accent-strong"
                                : "border-border",
                            )}
                          >
                            {checked ? "✓" : ""}
                          </span>
                          {app.icon && (
                            <img src={app.icon} alt="" className="size-4 shrink-0 rounded-[3px]" />
                          )}
                          <span className="min-w-0 flex-1 truncate text-text">{app.name}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          )}
          <p className="mt-2 text-[11px] text-text-faint">
            Changes rebuild the tap; apps must be playing to appear.
          </p>
        </div>
      )}
    </div>
  );
}

