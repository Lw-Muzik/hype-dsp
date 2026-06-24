import { useEffect, useMemo, useState } from "react";
import {
  CircleAlert,
  FileAudio,
  FolderOpen,
  Headphones,
  Orbit,
  Speaker,
  Square,
  Waves,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { Combobox } from "@/components/Combobox";
import type { ComboItem } from "@/components/Combobox";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  pickAudioFile,
  profileList,
  profileSetActive,
} from "@/lib/ipc";
import type { HeadphoneProfile, SpatialMode } from "@/lib/types";
import { cn } from "@/lib/cn";
import { Surround3DPanel } from "./Surround3DPanel";
import { RoomCard } from "./RoomCard";
import { ConvolverCard } from "./ConvolverCard";
import { CompanderCard } from "./CompanderCard";

export function EnhancerView() {
  const route = routeById("enhancer");

  const playing = useEngineStore((s) => s.playing);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const play = useEngineStore((s) => s.play);
  const stop = useEngineStore((s) => s.stop);
  const bass = useEngineStore((s) => s.state.bass);
  const spatializer = useEngineStore((s) => s.state.spatializer);
  const surround3d = useEngineStore((s) => s.state.surround3d);
  const headphone = useEngineStore((s) => s.state.headphone);
  const activeProfileId = useEngineStore((s) => s.state.activeProfileId);
  const setBass = useEngineStore((s) => s.setBass);
  const setSpatializer = useEngineStore((s) => s.setSpatializer);
  const setSurround3d = useEngineStore((s) => s.setSurround3d);
  const applyProfile = useEngineStore((s) => s.applyProfile);
  const clearProfile = useEngineStore((s) => s.clearProfile);

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [profiles, setProfiles] = useState<HeadphoneProfile[]>([]);
  const [surroundOpen, setSurroundOpen] = useState(false);

  const activeSpeakers = Object.values(surround3d.speakers).filter(Boolean).length;

  useEffect(() => {
    let cancelled = false;
    profileList()
      .then((list) => !cancelled && setProfiles(list))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const profileItems: ComboItem[] = useMemo(
    () => profiles.map((p) => ({ id: p.id, label: p.model, sublabel: p.brand })),
    [profiles],
  );
  const activeProfile = profiles.find((p) => p.id === activeProfileId) ?? null;

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

  const selectProfile = (id: string) => {
    profileSetActive(id)
      .then((p) => applyProfile(p))
      .catch(() => {});
  };

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="flex flex-col gap-4">
        {/* Audio source */}
        <Card title="Audio source" icon={FileAudio}>
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex min-h-[40px] items-center gap-3">
              {nowPlaying ? (
                <>
                  <span
                    className={cn(
                      "size-2.5 shrink-0 rounded-full",
                      playing ? "bg-success" : "bg-text-faint",
                    )}
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
                  Open a .wav to hear the chain. (More formats in Phase 5.)
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
          </div>
          {error && (
            <div className="mt-3 flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
              <CircleAlert
                className="mt-0.5 size-4 shrink-0 text-danger"
                aria-hidden="true"
              />
              <span>{error}</span>
            </div>
          )}
        </Card>

        <div className="grid gap-4 lg:grid-cols-2">
          {/* Bass */}
          <Card
            title="Bass boost"
            icon={Speaker}
            actions={
              <Switch
                checked={bass.enabled}
                onChange={(v) => setBass(v, bass.amount, bass.harmonics, bass.adaptive)}
                label="Enable bass boost"
              />
            }
          >
            <div className={cn("flex flex-col gap-4", !bass.enabled && "opacity-60")}>
              <div className="flex items-center gap-3">
                <span className="w-16 shrink-0 text-sm text-text-muted">Amount</span>
                <Slider
                  label="Bass amount"
                  min={0}
                  max={12}
                  step={0.5}
                  value={bass.amount}
                  onChange={(v) => setBass(bass.enabled, v, bass.harmonics, bass.adaptive)}
                  formatValue={(v) => `${v.toFixed(1)} decibels`}
                  className="flex-1"
                />
                <span className="w-12 text-right text-xs tabular-nums text-text-muted">
                  {bass.amount.toFixed(1)} dB
                </span>
              </div>
              <label className="flex items-center justify-between text-sm">
                <span className="text-text-muted">
                  Harmonic enhancement
                  <span className="ml-1 text-text-faint">(small drivers)</span>
                </span>
                <Switch
                  checked={bass.harmonics}
                  onChange={(v) => setBass(bass.enabled, bass.amount, v, bass.adaptive)}
                  label="Harmonic enhancement"
                />
              </label>
              <label className="flex items-center justify-between text-sm">
                <span className="text-text-muted">
                  Adaptive
                  <span className="ml-1 text-text-faint">(anti-overload)</span>
                </span>
                <Switch
                  checked={bass.adaptive}
                  onChange={(v) => setBass(bass.enabled, bass.amount, bass.harmonics, v)}
                  label="Adaptive bass (anti-overload)"
                />
              </label>
            </div>
          </Card>

          {/* Surround */}
          <Card
            title="Surround"
            icon={Waves}
            actions={
              <Switch
                checked={spatializer.enabled}
                onChange={(v) =>
                  setSpatializer(v, spatializer.amount, spatializer.mode)
                }
                label="Enable surround"
              />
            }
          >
            <div
              className={cn(
                "flex flex-col gap-4",
                !spatializer.enabled && "opacity-60",
              )}
            >
              <div className="flex items-center gap-3">
                <span className="w-16 shrink-0 text-sm text-text-muted">Amount</span>
                <Slider
                  label="Surround amount"
                  min={0}
                  max={1}
                  step={0.01}
                  value={spatializer.amount}
                  onChange={(v) =>
                    setSpatializer(spatializer.enabled, v, spatializer.mode)
                  }
                  formatValue={(v) => `${Math.round(v * 100)} percent`}
                  className="flex-1"
                />
                <span className="w-12 text-right text-xs tabular-nums text-text-muted">
                  {Math.round(spatializer.amount * 100)}%
                </span>
              </div>
              <div className="flex items-center gap-2">
                {(["crossfeed", "hrtf"] as const).map((m: SpatialMode) => (
                  <button
                    key={m}
                    type="button"
                    onClick={() =>
                      setSpatializer(spatializer.enabled, spatializer.amount, m)
                    }
                    className={cn(
                      "rounded-control border px-3 py-1.5 text-sm capitalize transition-colors",
                      spatializer.mode === m
                        ? "border-accent/40 bg-accent-muted text-accent-strong"
                        : "border-border text-text-muted hover:text-text",
                    )}
                  >
                    {m === "hrtf" ? "HRTF" : "Crossfeed"}
                  </button>
                ))}
              </div>
              <p className="text-xs text-text-faint">
                {spatializer.mode === "hrtf"
                  ? "HRTF: a parametric virtual-speaker model (inter-aural delay, level, pinna notch + widening) — places the image in front of and outside your head."
                  : "Crossfeed: a Bauer-style blend (delayed, head-shadowed bleed of the opposite channel) that relaxes hard-panned mixes on headphones."}
              </p>
            </div>
          </Card>
        </div>

        {/* 3D Surround */}
        <Card
          title="3D Surround"
          icon={Orbit}
          actions={
            <Switch
              checked={surround3d.enabled}
              onChange={(v) => setSurround3d({ ...surround3d, enabled: v })}
              label="Enable 3D Surround"
            />
          }
        >
          <div
            className={cn(
              "flex flex-wrap items-center justify-between gap-4",
              !surround3d.enabled && "opacity-60",
            )}
          >
            <p className="text-sm text-text-muted">
              Virtual {activeSpeakers}-speaker ring rendered binaurally —{" "}
              {Math.round(surround3d.intensity * 100)}% intensity, subwoofer{" "}
              {Math.round(surround3d.subwoofer * 100)}%.
            </p>
            <Button variant="secondary" onClick={() => setSurroundOpen(true)}>
              <Orbit className="size-4" aria-hidden="true" />
              Configure speakers
            </Button>
          </div>
        </Card>

        {/* Room reverb */}
        <RoomCard />

        {/* Convolver / IR correction */}
        <ConvolverCard />

        {/* Multiband compander */}
        <CompanderCard />

        {/* Headphone correction */}
        <Card title="Headphone correction" icon={Headphones}>
          <div className="flex flex-col gap-3">
            <Combobox
              items={profileItems}
              value={activeProfileId}
              onSelect={selectProfile}
              onClear={clearProfile}
              placeholder="Select your headphones…"
              searchPlaceholder="Search headphones…"
              emptyText="No matching headphones"
            />
            {headphone.enabled && activeProfile ? (
              <p className="text-xs text-text-muted">
                Correcting for <span className="text-text">{activeProfile.model}</span>{" "}
                — {headphone.bands.length} bands, preamp{" "}
                {headphone.preamp.toFixed(1)} dB.
              </p>
            ) : (
              <p className="text-xs text-text-faint">
                Genuine AutoEq (oratory1990) correction curves for{" "}
                {profiles.length} popular models.
              </p>
            )}
          </div>
        </Card>
      </div>

      <Surround3DPanel
        open={surroundOpen}
        onClose={() => setSurroundOpen(false)}
      />
    </div>
  );
}
