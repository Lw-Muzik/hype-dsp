import { useCallback, useEffect, useState } from "react";
import {
  AudioLines,
  CircleAlert,
  FolderPlus,
  Info,
  KeyRound,
  Library,
  ListMusic,
  RefreshCw,
  RotateCcw,
  Speaker,
  Sparkles,
  Wand2,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { DevicesView } from "@/features/devices/DevicesView";
import { CloudView } from "@/features/cloud/CloudView";
import { Button } from "@/components/Button";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import { useVisualizerStore, VISUALIZER_LIMITS } from "@/stores/visualizer";
import {
  captureVirtualAvailable,
  ipcErrorMessage,
  libraryIdentifyMissing,
  libraryList,
  libraryRefreshTags,
  libraryScan,
  onLibraryScanProgress,
  licenseDeactivate,
  licenseStatus,
  listOutputDevices,
  pickFolder,
  playerPlayCapture,
  playerPlaySystemAudio,
  stopSystemAudio,
  systemAudioAvailable,
} from "@/lib/ipc";
import type { DeviceInfo } from "@/lib/types";

function licenseLabel(
  license: ReturnType<typeof useUiStore.getState>["license"],
): string {
  if (!license) return "—";
  if (license.kind === "licensed") return "Licensed";
  if (license.kind === "expired") return "Trial expired";
  return `Trial — ${license.daysLeft} days left`;
}

type DeviceState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; devices: DeviceInfo[] };

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-2 text-sm">
      <span className="text-text-muted">{label}</span>
      <span className="font-medium tabular-nums">{value}</span>
    </div>
  );
}

/** A labelled slider row: fixed-width label · flexible track · value readout. */
function SliderRow({
  label,
  value,
  min,
  max,
  step,
  display,
  disabled = false,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  display: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-28 shrink-0 text-sm text-text-muted">{label}</span>
      <Slider
        label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={onChange}
        formatValue={() => display}
        className="flex-1"
      />
      <span className="w-14 text-right text-xs tabular-nums text-text-muted">
        {display}
      </span>
    </div>
  );
}

/**
 * Tunes the native MilkDrop visualizer sidecar — frame rate, beat reactivity,
 * and preset auto-cycling. Settings persist immediately; the sidecar reads its
 * config once at launch, so while the window is open a "Restart to apply" action
 * relaunches it with the new values. Hidden entirely when the sidecar isn't in
 * this build.
 */
function VisualizerCard() {
  const available = useVisualizerStore((s) => s.available);
  const running = useVisualizerStore((s) => s.running);
  const settings = useVisualizerStore((s) => s.settings);
  const update = useVisualizerStore((s) => s.update);
  const start = useVisualizerStore((s) => s.start);
  const probe = useVisualizerStore((s) => s.probe);

  // Probe on mount so the card appears even if the toolbar button isn't mounted.
  useEffect(() => {
    probe();
  }, [probe]);

  if (!available) return null;

  return (
    <Card
      title="Visualizer"
      icon={AudioLines}
      actions={
        running ? (
          <Button variant="secondary" onClick={() => void start()}>
            <RotateCcw className="size-4" aria-hidden="true" />
            Restart to apply
          </Button>
        ) : undefined
      }
    >
      <div className="flex flex-col gap-4">
        <p className="text-sm text-text-muted">
          The MilkDrop visualizer renders bundled presets that react to your
          audio in a separate window. Use{" "}
          <span className="text-text">&larr;</span> /{" "}
          <span className="text-text">&rarr;</span> in that window to change
          presets by hand.
        </p>

        <SliderRow
          label="Frame rate"
          min={VISUALIZER_LIMITS.fps.min}
          max={VISUALIZER_LIMITS.fps.max}
          step={VISUALIZER_LIMITS.fps.step}
          value={settings.fps}
          display={`${settings.fps} fps`}
          onChange={(v) => update({ fps: v })}
        />

        <SliderRow
          label="Beat sensitivity"
          min={VISUALIZER_LIMITS.beat.min}
          max={VISUALIZER_LIMITS.beat.max}
          step={VISUALIZER_LIMITS.beat.step}
          value={settings.beat}
          display={settings.beat.toFixed(1)}
          onChange={(v) => update({ beat: v })}
        />

        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0 text-sm">
            <p className="font-medium">Auto-cycle presets</p>
            <p className="text-xs text-text-muted">
              Move to the next preset automatically over time.
            </p>
          </div>
          <Switch
            checked={settings.autoCycle}
            onChange={(v) => update({ autoCycle: v })}
            label="Auto-cycle presets"
          />
        </div>

        <SliderRow
          label="Cycle every"
          min={VISUALIZER_LIMITS.cycleSecs.min}
          max={VISUALIZER_LIMITS.cycleSecs.max}
          step={VISUALIZER_LIMITS.cycleSecs.step}
          value={settings.cycleSecs}
          display={`${settings.cycleSecs}s`}
          disabled={!settings.autoCycle}
          onChange={(v) => update({ cycleSecs: v })}
        />
      </div>
    </Card>
  );
}

/**
 * Music library management — the one place music is imported. Scanning reads
 * each file's tags (title/artist/album/genre + cover art) into the library the
 * Player renders.
 */
function MusicLibraryCard() {
  const refreshLibrary = useLibraryStore((s) => s.refresh);
  const [count, setCount] = useState<number | null>(null);
  const [scanning, setScanning] = useState(false);
  const [op, setOp] = useState<"import" | "refresh" | "identify">("import");
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  const [note, setNote] = useState<string | null>(null);

  const loadCount = useCallback(() => {
    libraryList()
      .then((t) => setCount(t.length))
      .catch(() => setCount(null));
  }, []);

  useEffect(() => {
    loadCount();
  }, [loadCount]);

  // Reflect live scan progress so a large import never looks frozen.
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    onLibraryScanProgress((p) => setProgress(p))
      .then((fn) => (cancelled ? fn() : (un = fn)))
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  const addFolder = async () => {
    const dir = await pickFolder();
    if (!dir) return;
    setOp("import");
    setScanning(true);
    setNote(null);
    setProgress(null);
    try {
      const added = await libraryScan(dir);
      setNote(`Imported ${added} track${added === 1 ? "" : "s"}.`);
      loadCount();
      refreshLibrary();
    } catch (e) {
      setNote(`Scan failed: ${ipcErrorMessage(e)}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  };

  const refreshTags = async () => {
    if (!count) return;
    setOp("refresh");
    setScanning(true);
    setNote(null);
    setProgress(null);
    try {
      const n = await libraryRefreshTags();
      setNote(`Refreshed tags for ${n.toLocaleString()} track${n === 1 ? "" : "s"}.`);
      loadCount();
      refreshLibrary();
    } catch (e) {
      setNote(`Refresh failed: ${ipcErrorMessage(e)}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  };

  const identifyMissing = async () => {
    if (!count) return;
    setOp("identify");
    setScanning(true);
    setNote(null);
    setProgress(null);
    try {
      const n = await libraryIdentifyMissing();
      setNote(
        n === 0
          ? "No tracks could be identified."
          : `Identified and tagged ${n.toLocaleString()} track${n === 1 ? "" : "s"}.`,
      );
      loadCount();
      refreshLibrary();
    } catch (e) {
      setNote(`Identify failed: ${ipcErrorMessage(e)}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  };

  const progressLabel =
    op === "identify" ? "Identifying…" : op === "refresh" ? "Refreshing tags…" : "Importing…";

  const pct =
    progress && progress.total > 0
      ? Math.round((progress.done / progress.total) * 100)
      : null;

  return (
    <Card
      title="Music library"
      icon={Library}
      actions={
        <div className="flex gap-2">
          {count != null && count > 0 && (
            <>
              <Button variant="secondary" onClick={identifyMissing} disabled={scanning}>
                <Wand2 className="size-4" aria-hidden="true" />
                Identify missing
              </Button>
              <Button variant="secondary" onClick={refreshTags} disabled={scanning}>
                <RefreshCw className="size-4" aria-hidden="true" />
                Refresh tags
              </Button>
            </>
          )}
          <Button variant="primary" onClick={addFolder} disabled={scanning}>
            <FolderPlus className="size-4" aria-hidden="true" />
            {scanning ? "Scanning…" : "Add folder"}
          </Button>
        </div>
      }
    >
      <div className="flex flex-col gap-1">
        <p className="text-sm text-text-muted">
          Scan a folder to import its tracks. Titles, artists, albums, genres,
          and cover art are read from each file&rsquo;s tags and shown in the
          Player. If a library scanned earlier shows filenames instead of tags,
          use <span className="text-text">Refresh tags</span>.{" "}
          <span className="text-text">Identify missing</span> fingerprints tracks
          without artist/title info and fills them in from AcoustID.
        </p>
        <div className="divide-y divide-border">
          <InfoRow
            label="Tracks in library"
            value={count == null ? "—" : count.toLocaleString()}
          />
        </div>
        {scanning && progress && (
          <div className="flex flex-col gap-1.5 pt-1">
            <div className="flex items-center justify-between text-xs text-text-muted">
              <span>{progressLabel}</span>
              <span className="tabular-nums">
                {progress.done.toLocaleString()} / {progress.total.toLocaleString()}
                {pct != null ? ` · ${pct}%` : ""}
              </span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-border-strong">
              <div
                className="h-full rounded-full bg-accent transition-[width] duration-150"
                style={{ width: `${pct ?? 0}%` }}
              />
            </div>
          </div>
        )}
        {!scanning && note && <p className="text-xs text-text-faint">{note}</p>}
      </div>
    </Card>
  );
}

function OutputDevices({ state }: { state: DeviceState }) {
  if (state.status === "loading") {
    return (
      <div className="space-y-2" aria-busy="true" aria-label="Loading devices">
        {[0, 1, 2].map((i) => (
          <div
            key={i}
            className="h-9 animate-pulse rounded-control bg-surface-overlay"
          />
        ))}
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div className="flex items-center gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2.5 text-sm text-text">
        <CircleAlert className="size-4 shrink-0 text-danger" aria-hidden="true" />
        <span>Could not list devices: {state.message}</span>
      </div>
    );
  }

  if (state.devices.length === 0) {
    return <p className="text-sm text-text-muted">No output devices found.</p>;
  }

  return (
    <ul className="divide-y divide-border">
      {state.devices.map((device) => (
        <li
          key={device.name}
          className="flex items-center justify-between py-2.5 text-sm"
        >
          <span className="truncate">{device.name}</span>
          {device.isDefault && (
            <span className="ml-3 shrink-0 rounded-full border border-accent/40 bg-accent-muted px-2 py-0.5 text-xs text-accent-strong">
              Default
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}

/**
 * Settings — the one Phase 0 view backed by live data: it shows real app
 * metadata (from `app_info`) and the system's real output devices (from
 * `audio_list_output_devices`), proving the typed IPC seam at runtime.
 */
export function SettingsView() {
  const route = routeById("settings");
  const appInfo = useUiStore((s) => s.appInfo);
  const license = useUiStore((s) => s.license);
  const setLicense = useUiStore((s) => s.setLicense);
  const playback = useEngineStore((s) => s.state.playback);
  const setPlayback = useEngineStore((s) => s.setPlayback);
  const [devices, setDevices] = useState<DeviceState>({ status: "loading" });
  const [, setVirtualAvailable] = useState(false);
  const [systemAvailable, setSystemAvailable] = useState(false);
  // System-wide EQ runs out-of-band on Linux/Windows (not through the engine's
  // play state), so track its on/off here for the toggle.
  const [systemEqOn, setSystemEqOn] = useState(false);
  const [captureError, setCaptureError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    listOutputDevices()
      .then((list) => {
        if (!cancelled) setDevices({ status: "ready", devices: list });
      })
      .catch((err: unknown) => {
        if (!cancelled)
          setDevices({ status: "error", message: ipcErrorMessage(err) });
      });
    captureVirtualAvailable()
      .then((v) => !cancelled && setVirtualAvailable(v))
      .catch(() => {});
    systemAudioAvailable()
      .then((v) => !cancelled && setSystemAvailable(v))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const startCapture = () => {
    setCaptureError(null);
    playerPlayCapture().catch((e) => setCaptureError(ipcErrorMessage(e)));
  };
  const startSystemAudio = () => {
    setCaptureError(null);
    playerPlaySystemAudio()
      .then(() => setSystemEqOn(true))
      .catch((e) => setCaptureError(ipcErrorMessage(e)));
  };
  const stopSystemEq = () => {
    stopSystemAudio()
      .catch(() => {})
      .finally(() => setSystemEqOn(false));
  };
  const deactivate = () => {
    licenseDeactivate()
      .then(() => licenseStatus())
      .then(setLicense)
      .catch(() => {});
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      <PageHeader
        icon={route.icon}
        title={route.label}
        subtitle={route.tagline}
      />
      <div className="grid gap-4">
        <Card title="About" icon={Info}>
          <div className="divide-y divide-border">
            <InfoRow label="Application" value={appInfo?.name ?? "HypeMuzik"} />
            <InfoRow label="Version" value={appInfo?.version ?? "—"} />
            <InfoRow
              label="Engine schema"
              value={appInfo ? `v${appInfo.engineSchema}` : "—"}
            />
          </div>
        </Card>

        <MusicLibraryCard />

        <DevicesView />

        <CloudView />

        <Card
          title="Playback"
          icon={ListMusic}
          actions={
            <Switch
              checked={playback.gapless}
              onChange={(v) => setPlayback(v, playback.crossfadeSecs)}
              label="Gapless playback"
            />
          }
        >
          <div className="flex flex-col gap-4">
            <p className="text-sm text-text-muted">
              Gapless removes silence between tracks. Crossfade overlaps the end
              of one track with the start of the next (any crossfade implies
              gapless). Applies to the next track list you play.
            </p>
            <div className="flex items-center gap-3">
              <span className="w-20 shrink-0 text-sm text-text-muted">
                Crossfade
              </span>
              <Slider
                label="Crossfade duration"
                min={0}
                max={12}
                step={0.5}
                value={playback.crossfadeSecs}
                onChange={(v) => setPlayback(playback.gapless, v)}
                formatValue={(v) =>
                  v === 0 ? "off" : `${v.toFixed(1)} seconds`
                }
                className="flex-1"
              />
              <span className="w-12 text-right text-xs tabular-nums text-text-muted">
                {playback.crossfadeSecs === 0
                  ? "Off"
                  : `${playback.crossfadeSecs.toFixed(1)}s`}
              </span>
            </div>
          </div>
        </Card>

        <Card title="Output devices" icon={Speaker}>
          <OutputDevices state={devices} />
        </Card>

        <Card title="Account" icon={KeyRound}>
          <div className="divide-y divide-border">
            <InfoRow label="License (mock)" value={licenseLabel(license)} />
          </div>
          {license?.kind === "licensed" && (
            <div className="mt-3 flex justify-end">
              <Button variant="secondary" onClick={deactivate}>
                Deactivate
              </Button>
            </div>
          )}
          <p className="mt-3 text-xs text-text-faint">
            Licensing is an explicitly-marked local mock — no real DRM or
            activation server. See docs/architecture.md for the production
            contract.
          </p>
        </Card>

        <Card title="System-wide audio" icon={Sparkles}>
          <div className="flex flex-col gap-3">
            {systemAvailable && (
              <div className="flex items-center justify-between gap-3 rounded-control border border-accent/30 bg-accent-muted/40 px-3 py-2.5">
                <div className="min-w-0 text-sm">
                  <p className="font-medium text-accent-strong">
                    Equalize everything you hear
                  </p>
                  <p className="text-xs text-text-muted">
                    Routes all system audio through the equalizer and effects.
                  </p>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button variant="primary" onClick={startSystemAudio}>
                    {systemEqOn ? "Restart" : "Enable"}
                  </Button>
                  {systemEqOn && (
                    <Button variant="secondary" onClick={stopSystemEq}>
                      Stop
                    </Button>
                  )}
                </div>
              </div>
            )}

            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0 text-sm">
                <p className="font-medium">Input capture</p>
                <p className="text-xs text-text-muted">
                  Route a microphone / input device through the chain.
                </p>
              </div>
              <Button variant="secondary" onClick={startCapture}>
                Start
              </Button>
            </div>

            {captureError && (
              <p className="text-sm text-danger">{captureError}</p>
            )}

            <div className="flex items-start gap-2 rounded-control border border-border bg-surface px-3 py-2 text-xs text-text-muted">
              <CircleAlert
                className="mt-0.5 size-3.5 shrink-0 text-text-faint"
                aria-hidden="true"
              />
              <span>
                {systemAvailable
                  ? "Everything you hear is re-rendered through the chain. macOS taps other apps (first use prompts for audio-capture permission; the grant persists on a code-signed build); Linux routes through a PipeWire/PulseAudio virtual sink and restores your default output when stopped."
                  : "System-wide equalization isn't available here — on Windows it needs the HypeMuzik virtual audio device. See docs/system-eq.md."}
              </span>
            </div>
          </div>
        </Card>

        <VisualizerCard />
      </div>
    </div>
  );
}
