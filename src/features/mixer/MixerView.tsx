import { useCallback, useEffect, useRef, useState } from "react";
import { Info, Volume2, VolumeX } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Slider } from "@/components/Slider";
import { ipcErrorMessage, mixerListSessions, mixerSetMuted, mixerSetVolume } from "@/lib/ipc";
import { toast } from "@/stores/toast";
import type { AppSession, MixerSnapshot } from "@/lib/types";

export function MixerView() {
  const route = routeById("mixer");
  const [snap, setSnap] = useState<MixerSnapshot | null>(null);

  const refresh = useCallback(() => {
    mixerListSessions()
      .then(setSnap)
      .catch(() =>
        setSnap({
          supported: false,
          unavailableReason: "The mixer is unavailable.",
          sessions: [],
        }),
      );
  }, []);

  useEffect(() => {
    refresh();
    // The window hides (not quits) on close: skip polls while hidden and
    // refresh immediately when the window becomes visible again.
    const t = setInterval(() => {
      if (!document.hidden) refresh();
    }, 3000);
    const onVisible = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(t);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  // A failed set (almost always: audio-capture not granted, or an unsigned
  // build where macOS never prompts) used to fail silently — the slider moved
  // but nothing happened. Surface it, throttled, since a dragged slider fires a
  // burst of calls that would otherwise spam identical toasts.
  const lastErrorAt = useRef(0);
  const reportSetError = (e: unknown) => {
    const now = Date.now();
    if (now - lastErrorAt.current < 5000) return;
    lastErrorAt.current = now;
    toast.error(ipcErrorMessage(e));
  };

  const patch = (id: string, change: Partial<AppSession>) =>
    setSnap((s) =>
      s
        ? {
            ...s,
            sessions: s.sessions.map((x) =>
              x.id === id ? { ...x, ...change } : x,
            ),
          }
        : s,
    );

  return (
    <div className="mx-auto w-full max-w-3xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      {snap && !snap.supported ? (
        <Card title="Per-app volume" icon={Info}>
          <div className="flex items-start gap-3 text-sm">
            <Info className="mt-0.5 size-4 shrink-0 text-text-muted" aria-hidden="true" />
            <p className="text-text-muted">
              {snap.unavailableReason ??
                "Per-application volume isn't available on this platform."}
            </p>
          </div>
        </Card>
      ) : snap && snap.sessions.length === 0 ? (
        <Card title="Applications" icon={Volume2}>
          <p className="text-sm text-text-muted">
            No applications are playing audio right now.
          </p>
        </Card>
      ) : (
        <Card title="Applications" icon={Volume2}>
          <ul className="divide-y divide-border">
            {(snap?.sessions ?? []).map((s) => (
              <li key={s.id} className="flex items-center gap-3 py-3">
                {s.icon ? (
                  <img
                    src={s.icon}
                    alt=""
                    aria-hidden="true"
                    className="size-7 shrink-0 rounded-md"
                  />
                ) : (
                  <span
                    aria-hidden="true"
                    className="grid size-7 shrink-0 place-items-center rounded-md bg-surface-overlay"
                  >
                    <Volume2 className="size-4 text-text-faint" />
                  </span>
                )}
                <span className="w-36 shrink-0 truncate text-sm font-medium">
                  {s.name}
                </span>
                <button
                  type="button"
                  aria-label={s.muted ? "Unmute" : "Mute"}
                  onClick={() => {
                    patch(s.id, { muted: !s.muted });
                    void mixerSetMuted(s.id, !s.muted).catch(reportSetError);
                  }}
                  className="text-text-muted hover:text-text"
                >
                  {s.muted ? (
                    <VolumeX className="size-4 text-danger" aria-hidden="true" />
                  ) : (
                    <Volume2 className="size-4" aria-hidden="true" />
                  )}
                </button>
                <Slider
                  label={`${s.name} volume`}
                  min={0}
                  max={1}
                  step={0.01}
                  value={s.volume}
                  onChange={(v) => {
                    patch(s.id, { volume: v });
                    void mixerSetVolume(s.id, v).catch(reportSetError);
                  }}
                  className="flex-1"
                />
                <span className="w-10 text-right text-xs tabular-nums text-text-muted">
                  {Math.round(s.volume * 100)}%
                </span>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
