import { useCallback, useEffect, useState } from "react";
import { Info, Volume2, VolumeX } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Slider } from "@/components/Slider";
import { mixerListSessions, mixerSetMuted, mixerSetVolume } from "@/lib/ipc";
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
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);

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
                <span className="w-40 shrink-0 truncate text-sm font-medium">
                  {s.name}
                </span>
                <button
                  type="button"
                  aria-label={s.muted ? "Unmute" : "Mute"}
                  onClick={() => {
                    patch(s.id, { muted: !s.muted });
                    void mixerSetMuted(s.id, !s.muted).catch(() => {});
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
                    void mixerSetVolume(s.id, v).catch(() => {});
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
