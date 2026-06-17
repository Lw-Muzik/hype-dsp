import { useState } from "react";
import {
  CircleAlert,
  FileAudio,
  FolderOpen,
  Power,
  Sparkles,
  Square,
  Volume2,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import { ipcErrorMessage, pickAudioFile } from "@/lib/ipc";

/**
 * Enhancer — the Phase 2 surface. Plays a local file through the live DSP chain
 * so power/master-volume changes are audible. The full at-a-glance enhancer
 * (surround/bass dials, big meters) fills in over later phases.
 */
export function EnhancerView() {
  const route = routeById("enhancer");

  const playing = useEngineStore((s) => s.playing);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const play = useEngineStore((s) => s.play);
  const stop = useEngineStore((s) => s.stop);
  const power = useEngineStore((s) => s.state.power);
  const masterVolume = useEngineStore((s) => s.state.masterVolume);

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const handleOpen = async () => {
    setError(null);
    const path = await pickAudioFile();
    if (!path) return;
    const name = path.split(/[\\/]/).pop() ?? path;
    setBusy(true);
    try {
      await play(path, name);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="Audio source" icon={FileAudio}>
          <div className="flex flex-col gap-4">
            <div className="flex min-h-[44px] items-center gap-3">
              {nowPlaying ? (
                <>
                  <span
                    className={cnDot(playing)}
                    aria-hidden="true"
                  />
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{nowPlaying}</p>
                    <p className="text-xs text-text-muted">
                      {playing ? "Playing through the chain" : "Finished"}
                    </p>
                  </div>
                </>
              ) : (
                <p className="text-sm text-text-muted">
                  No file loaded. Open a .wav to hear the engine.
                </p>
              )}
            </div>

            <div className="flex items-center gap-2">
              <Button variant="primary" onClick={handleOpen} disabled={busy}>
                <FolderOpen className="size-4" aria-hidden="true" />
                {busy ? "Opening…" : "Open file"}
              </Button>
              {playing && (
                <Button variant="secondary" onClick={() => void stop()}>
                  <Square className="size-4" aria-hidden="true" />
                  Stop
                </Button>
              )}
            </div>

            {error && (
              <div className="flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
                <CircleAlert
                  className="mt-0.5 size-4 shrink-0 text-danger"
                  aria-hidden="true"
                />
                <span>{error}</span>
              </div>
            )}

            <p className="text-xs text-text-faint">
              Phase 2 plays WAV files. MP3, FLAC, AAC and OGG arrive with the
              media subsystem in Phase 5.
            </p>
          </div>
        </Card>

        <Card title="Enhancement" icon={Sparkles}>
          <div className="flex flex-col divide-y divide-border">
            <div className="flex items-center justify-between py-2.5 text-sm">
              <span className="flex items-center gap-2 text-text-muted">
                <Power className="size-4" aria-hidden="true" />
                Power
              </span>
              <span
                className={
                  power ? "font-medium text-accent-strong" : "text-text-muted"
                }
              >
                {power ? "Engaged" : "Bypassed"}
              </span>
            </div>
            <div className="flex items-center justify-between py-2.5 text-sm">
              <span className="flex items-center gap-2 text-text-muted">
                <Volume2 className="size-4" aria-hidden="true" />
                Master volume
              </span>
              <span className="font-medium tabular-nums">
                {Math.round(masterVolume * 100)}%
              </span>
            </div>
          </div>
          <p className="mt-3 text-xs text-text-faint">
            Power and master volume are live (top bar). The 31-band EQ, bass,
            and surround dials fill in across Phases 3–4.
          </p>
        </Card>
      </div>
    </div>
  );
}

function cnDot(active: boolean): string {
  return [
    "size-2.5 shrink-0 rounded-full",
    active ? "bg-success" : "bg-text-faint",
  ].join(" ");
}
