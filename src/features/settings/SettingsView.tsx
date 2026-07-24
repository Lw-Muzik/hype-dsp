import { useCallback, useEffect, useState } from "react";
import {
  Airplay,
  AudioLines,
  Cast,
  Check,
  CircleAlert,
  FolderPlus,
  Headphones,
  Info,
  KeyRound,
  Library,
  ListMusic,
  LogOut,
  MonitorSpeaker,
  RefreshCw,
  RotateCcw,
  Speaker,
  Sparkles,
  Usb,
  Wand2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { DevicesView } from "@/features/devices/DevicesView";
import { CloudView } from "@/features/cloud/CloudView";
import ThemeCard from "@/features/settings/ThemeCard";
import { UpdateRow } from "@/features/settings/UpdateRow";
import { YtMusicView } from "@/features/ytmusic/YtMusicView";
import { Button } from "@/components/Button";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import { useVisualizerStore, VISUALIZER_LIMITS } from "@/stores/visualizer";
import { useSystemEqStore } from "@/stores/systemEq";
import { useAccountStore } from "@/stores/account";
import { toast } from "@/stores/toast";
import {
  captureVirtualAvailable,
  ipcErrorMessage,
  libraryCount,
  libraryIdentifyMissing,
  libraryRefreshTags,
  libraryScan,
  onLibraryScanProgress,
  outputDevices,
  setDefaultOutput,
  pickFolder,
  playerPlayCapture,
  mixerListSessions,
  systemAudioStatus,
  systemAudioInstallDriver,
  apoSetup,
  systemEqStatus,
  type RoutingSetupPhase,
  type SystemAudioStatus,
  type SystemEqRuntimeStatus,
} from "@/lib/ipc";
import { listen } from "@tauri-apps/api/event";
import { systemAudioAffordance } from "./systemAudioCard";

/** Set once the routing setup installed VB-CABLE but Windows still needs a
 *  restart before the device enumerates; cleared when the device appears. */
const SETUP_REBOOT_KEY = "hm.systemEqSetupAwaitingReboot";
import type {
  AppSession,
  LicenseInfo,
  OutputDevice,
  OutputTransport,
  SystemEqScopeMode,
} from "@/lib/types";

/** Human date (e.g. "14 Jul 2026") for a server ISO timestamp, or null. */
function formatDate(iso: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/** Label + badge colour for each license state. */
const PLAN_META: Record<LicenseInfo["state"], { label: string; cls: string }> = {
  trial: { label: "Trial", cls: "bg-amber-500/15 text-amber-400" },
  licensed: { label: "Licensed", cls: "bg-success/15 text-success" },
  expired: { label: "Expired", cls: "bg-danger/15 text-danger" },
  blocked: { label: "Blocked", cls: "bg-danger/15 text-danger" },
};

type DeviceState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; devices: OutputDevice[] };

/** Icon for a device's physical transport (coarse speaker/headphone hint). */
const TRANSPORT_ICON: Record<OutputTransport, LucideIcon> = {
  builtin: Speaker,
  usb: Usb,
  bluetooth: Headphones,
  hdmi: MonitorSpeaker,
  displayport: MonitorSpeaker,
  airplay: Airplay,
  aggregate: AudioLines,
  virtual: AudioLines,
  thunderbolt: Cast,
  other: Speaker,
};

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
      title="Fullscreen visualizer"
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
          These apply to the visualizer window opened from the Visuals view. Pick
          which preset it shows over there; use{" "}
          <span className="text-text">&larr;</span> /{" "}
          <span className="text-text">&rarr;</span> in the window to browse by
          hand.
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
    // Just the count — never pull the whole library here (a 20k+ drive would
    // make this a multi-MB transfer + parse just to show a number).
    libraryCount()
      .then((n) => setCount(n))
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
      .catch(() => { });
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

/**
 * Selectable output-device picker. Clicking a device makes it the system default
 * output; the engine follows the default, so the whole app's audio moves with it
 * (like macOS Sound settings). The active device is highlighted; failures surface
 * as a toast and leave the previous selection intact.
 */
function OutputDevices({
  state,
  switchingUid,
  onSelect,
}: {
  state: DeviceState;
  switchingUid: string | null;
  onSelect: (device: OutputDevice) => void;
}) {
  if (state.status === "loading") {
    return (
      <div className="space-y-2" aria-busy="true" aria-label="Loading devices">
        {[0, 1, 2].map((i) => (
          <div
            key={i}
            className="h-11 animate-pulse rounded-control bg-surface-overlay"
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
    <ul className="flex flex-col gap-1.5" role="radiogroup" aria-label="Output device">
      {state.devices.map((device) => {
        const Icon = TRANSPORT_ICON[device.transport] ?? Speaker;
        const active = device.isDefault;
        const busy = switchingUid === device.uid;
        return (
          <li key={device.uid}>
            <button
              type="button"
              role="radio"
              aria-checked={active}
              disabled={busy}
              onClick={() => !active && onSelect(device)}
              className={`flex w-full items-center gap-3 rounded-control border px-3 py-2.5 text-left text-sm transition-colors disabled:opacity-60 ${active
                  ? "border-accent/50 bg-accent-muted"
                  : "border-border bg-surface hover:border-border-strong hover:bg-surface-overlay"
                }`}
            >
              <Icon
                className={`size-4 shrink-0 ${active ? "text-accent-strong" : "text-text-muted"}`}
                aria-hidden="true"
              />
              <span className={`min-w-0 flex-1 truncate ${active ? "font-medium text-accent-strong" : ""}`}>
                {device.name}
              </span>
              {busy ? (
                <RefreshCw
                  className="size-4 shrink-0 animate-spin text-text-muted"
                  aria-hidden="true"
                />
              ) : active ? (
                <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-accent/40 bg-accent-muted px-2 py-0.5 text-xs text-accent-strong">
                  <Check className="size-3" aria-hidden="true" />
                  Active
                </span>
              ) : null}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/**
 * Settings — backed by live data: real app metadata (from `app_info`) and the
 * system's real output devices (from `audio_output_devices`), which are also
 * selectable to switch the system default output (`audio_set_default_output`).
 */
export function SettingsView() {
  const route = routeById("settings");
  const appInfo = useUiStore((s) => s.appInfo);
  const account = useAccountStore((s) => s.status);
  const logout = useAccountStore((s) => s.logout);
  const playback = useEngineStore((s) => s.state.playback);
  const setPlayback = useEngineStore((s) => s.setPlayback);
  const setDataSaver = useEngineStore((s) => s.setDataSaver);
  const [devices, setDevices] = useState<DeviceState>({ status: "loading" });
  const [switchingUid, setSwitchingUid] = useState<string | null>(null);
  const [, setVirtualAvailable] = useState(false);
  const [systemStatus, setSystemStatus] = useState<SystemAudioStatus>({
    supported: false,
    available: false,
    driverInstalled: false,
    needsDriver: false,
    driverBundled: false,
    apoInstalled: false,
  });
  const [driverInstalling, setDriverInstalling] = useState(false);
  const [driverError, setDriverError] = useState<string | null>(null);
  // Non-null while the one-click routing setup runs; the value is the phase the
  // backend last reported, so the button narrates real progress.
  const [setupPhase, setSetupPhase] = useState<RoutingSetupPhase | null>(null);
  const [awaitingReboot, setAwaitingReboot] = useState(
    () => localStorage.getItem(SETUP_REBOOT_KEY) === "1",
  );
  // Which affordance the card shows: Enable, Install driver, one-click setup
  // ("not-bundled" = a Windows build shipped without the signed driver), or an
  // honest unavailable notice.
  const systemAffordance = systemAudioAffordance(systemStatus);
  // Live engine truth for system-wide EQ (distinct from the persisted *intent*
  // below): `recovering` means a transient failure — e.g. a macOS tap stall under
  // heavy load or a device change — is being recovered in the background, so the
  // card can say so instead of the EQ appearing to have silently stopped.
  const [runtimeStatus, setRuntimeStatus] =
    useState<SystemEqRuntimeStatus>("disabled");
  // System-wide EQ is a persisted session mode (re-engaged on launch), so its
  // on/off + last error live in a shared store rather than local state.
  const systemEqOn = useSystemEqStore((s) => s.enabled);
  const systemEqError = useSystemEqStore((s) => s.error);
  const enableSystemEq = useSystemEqStore((s) => s.enable);
  const disableSystemEq = useSystemEqStore((s) => s.disable);
  const [captureError, setCaptureError] = useState<string | null>(null);

  // Reload the output-device list. Kept in a ref-stable callback so it can be
  // called on mount, on a poll (to catch hot-plugged / removed devices), and
  // after switching the default. The list is a fresh snapshot each time, so we
  // never leave the error state stuck once a device reappears.
  const loadDevices = useCallback(async () => {
    try {
      const list = await outputDevices();
      setDevices({ status: "ready", devices: list });
    } catch (err) {
      setDevices({ status: "error", message: ipcErrorMessage(err) });
    }
  }, []);

  // Per-app system-EQ selection is a macOS-only capability (the process tap);
  // Linux/Windows re-route the whole output, so the scope picker is hidden there.
  const isMacos = navigator.userAgent.toLowerCase().includes("mac");
  const systemEqScope = useEngineStore((s) => s.state.systemEqScope);
  const setSystemEqScope = useEngineStore((s) => s.setSystemEqScope);
  const [scopeApps, setScopeApps] = useState<AppSession[]>([]);

  const refreshScopeApps = useCallback(() => {
    if (!isMacos) return;
    mixerListSessions()
      .then((snap) => setScopeApps(snap.sessions))
      .catch(() => {});
  }, [isMacos]);

  useEffect(() => {
    let cancelled = false;
    void loadDevices();
    // Poll for hot-plug / removal + external default-device changes while the
    // view is open (no Core Audio listener needed for this cadence).
    const id = window.setInterval(() => void loadDevices(), 3000);
    captureVirtualAvailable()
      .then((v) => !cancelled && setVirtualAvailable(v))
      .catch(() => { });
    systemAudioStatus()
      .then((s) => !cancelled && setSystemStatus(s))
      .catch(() => { });
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [loadDevices]);

  // Make a device the system default output. The engine follows the default,
  // so this moves all of the app's (and the system's) audio to it.
  const selectDevice = useCallback(
    async (device: OutputDevice) => {
      setSwitchingUid(device.uid);
      try {
        await setDefaultOutput(device.uid);
        await loadDevices();
      } catch (err) {
        toast.error(`Couldn't switch to ${device.name}: ${ipcErrorMessage(err)}`);
        // Re-sync so the highlight reflects whatever the system actually did.
        await loadDevices();
      } finally {
        setSwitchingUid(null);
      }
    },
    [loadDevices],
  );

  // Narrate the one-click routing setup: the backend emits download → install
  // → detect phases so the button can show what is actually happening.
  useEffect(() => {
    const unlisten = listen<RoutingSetupPhase>("system-eq-setup-phase", (e) =>
      setSetupPhase(e.payload),
    );
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  // A routing device that has appeared clears any pending "restart to finish
  // setup" notice (the reboot happened, or the device enumerated late).
  useEffect(() => {
    if (systemStatus.available && awaitingReboot) {
      localStorage.removeItem(SETUP_REBOOT_KEY);
      setAwaitingReboot(false);
    }
  }, [systemStatus.available, awaitingReboot]);

  // Poll the live system-EQ runtime status while the Settings view is open, so a
  // background recovery (or a settled "active"/"disabled") is reflected promptly.
  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      systemEqStatus()
        .then((s) => !cancelled && setRuntimeStatus(s))
        .catch(() => { });
    };
    poll();
    const id = window.setInterval(poll, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const startCapture = () => {
    setCaptureError(null);
    playerPlayCapture().catch((e) => setCaptureError(ipcErrorMessage(e)));
  };
  const startSystemAudio = () => {
    // Refresh the running-app list on the way in, so the scope picker below
    // isn't empty the first time it's opened.
    refreshScopeApps();
    void enableSystemEq();
  };
  const stopSystemEq = () => {
    void disableSystemEq();
  };
  // Install the bundled Windows audio driver (UAC prompt), then poll status until
  // the virtual device enumerates (Plug-and-Play can lag the installer's exit).
  const installAudioDriver = async () => {
    setDriverError(null);
    setDriverInstalling(true);
    try {
      await systemAudioInstallDriver();
      for (let i = 0; i < 6; i++) {
        await new Promise((r) => setTimeout(r, 700));
        const next = await systemAudioStatus();
        setSystemStatus(next);
        if (next.driverInstalled) break;
      }
    } catch (e) {
      setDriverError(ipcErrorMessage(e));
    } finally {
      setDriverInstalling(false);
    }
  };
  // One-click Windows routing setup (VB-CABLE download + verified silent
  // install), then auto-enable — the click should end with equalized audio,
  // not another button.
  // The free, white-label Windows path: install our own APO (one UAC prompt +
  // reboot). Nothing third-party for the user to see, no cert/account/card.
  const setupApo = async () => {
    setDriverError(null);
    setSetupPhase("installing");
    try {
      const outcome = await apoSetup();
      const next = await systemAudioStatus();
      setSystemStatus(next);
      if (outcome === "needsReboot") {
        localStorage.setItem(SETUP_REBOOT_KEY, "1");
        setAwaitingReboot(true);
      } else if (next.available) {
        startSystemAudio();
      }
    } catch (e) {
      setDriverError(ipcErrorMessage(e));
    } finally {
      setSetupPhase(null);
    }
  };

  // Apply a scope change. The scope is committed to the backend FIRST (awaited),
  // then — if the tap is running — rebuilt so the new scope takes effect.
  //
  // The rebuild goes through the system-EQ store rather than calling
  // `playerPlaySystemAudio` directly, because that store is now what owns the
  // on/off state. It already does what this needed doing by hand: on a failed
  // rebuild — an empty allowlist because the picked apps went idle — it stops,
  // records the error and clears `enabled`, so the toggle matches the stopped
  // tap instead of re-erroring.
  const applyScope = async (next: { mode: SystemEqScopeMode; apps: string[] }) => {
    setCaptureError(null);
    try {
      await setSystemEqScope(next);
    } catch (e) {
      setCaptureError(ipcErrorMessage(e));
      return;
    }
    if (systemEqOn) await enableSystemEq();
  };
  const setScopeMode = (mode: SystemEqScopeMode) => {
    refreshScopeApps();
    void applyScope({ mode, apps: systemEqScope.apps });
  };
  const toggleScopeApp = (id: string) => {
    const apps = systemEqScope.apps.includes(id)
      ? systemEqScope.apps.filter((a) => a !== id)
      : [...systemEqScope.apps, id];
    void applyScope({ mode: systemEqScope.mode, apps });
  };
  // Renewal date + label, from the server license (licensed → renewal date,
  // otherwise the trial end date).
  const lic = account?.license ?? null;
  const renewDate = lic
    ? formatDate(lic.state === "licensed" ? lic.licensedUntil : lic.trialEndsAt)
    : null;
  const renewInfo = renewDate
    ? { label: lic!.state === "licensed" ? "Renews on" : "Trial ends", value: renewDate }
    : null;

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
            <div className="border-t border-border pt-3">
              <UpdateRow />
            </div>
            <InfoRow
              label="Engine schema"
              value={appInfo ? `v${appInfo.engineSchema}` : "—"}
            />
          </div>
        </Card>

        <ThemeCard />

        <MusicLibraryCard />

        <DevicesView />

        <CloudView />
        {/* youtube music view */}
        <YtMusicView />

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
            {/* Data Saver silently wins over this slider for everything that
                streams, which reads as "crossfade is broken for cloud and
                YouTube" — the setting is doing exactly what it says, two screens
                away from where the effect shows up. Say so where it's set. */}
            {playback.dataSaver && playback.crossfadeSecs > 0 && (
              <p className="flex items-start gap-2 rounded-control bg-surface-overlay px-3 py-2 text-xs text-text-muted">
                <CircleAlert
                  className="mt-0.5 size-3.5 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <span>
                  Data Saver is on, so crossfade applies to local files only.
                  Cloud, phone and YouTube Music play one track at a time and cut
                  straight from one to the next.
                </span>
              </p>
            )}
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0 text-sm">
                <p className="font-medium">Data Saver</p>
                <p className="text-xs text-text-muted">
                  Stream progressively on slow connections (no full-download /
                  prefetch). Turns off gapless and crossfade for cloud, phone and
                  YouTube Music.
                </p>
              </div>
              <Switch
                checked={playback.dataSaver}
                onChange={(v) => setDataSaver(v)}
                label="Data Saver"
              />
            </div>
          </div>
        </Card>

        <Card title="Output devices" icon={Speaker}>
          <div className="flex flex-col gap-3">
            <p className="text-sm text-text-muted">
              Choose where sound plays. Selecting a device makes it the system
              default output, so everything — this app and the rest of macOS —
              follows.
            </p>
            <OutputDevices
              state={devices}
              switchingUid={switchingUid}
              onSelect={(d) => void selectDevice(d)}
            />
          </div>
        </Card>

        <Card title="Account" icon={KeyRound}>
          {account?.authenticated ? (
            <>
              <div className="divide-y divide-border">
                {account.email && <InfoRow label="Email" value={account.email} />}
                {account.name && <InfoRow label="Name" value={account.name} />}
                <div className="flex items-center justify-between py-2 text-sm">
                  <span className="text-text-muted">Plan</span>
                  {lic ? (
                    <span
                      className={`rounded-full px-2 py-0.5 text-xs font-medium ${PLAN_META[lic.state].cls}`}
                    >
                      {PLAN_META[lic.state].label}
                    </span>
                  ) : (
                    <span className="font-medium">—</span>
                  )}
                </div>
                {lic && (
                  <InfoRow
                    label="Days remaining"
                    value={`${lic.daysLeft} day${lic.daysLeft === 1 ? "" : "s"}`}
                  />
                )}
                {renewInfo && <InfoRow label={renewInfo.label} value={renewInfo.value} />}
              </div>
              <div className="mt-3 flex items-center justify-end gap-2">
                <Button
                  variant="secondary"
                  onClick={() => toast.info("Renewals are coming soon.")}
                >
                  <RefreshCw className="size-4" aria-hidden="true" />
                  Renew
                </Button>
                <Button variant="ghost" onClick={() => void logout()}>
                  <LogOut className="size-4" aria-hidden="true" />
                  Sign out
                </Button>
              </div>
            </>
          ) : (
            <p className="py-2 text-sm text-text-muted">You’re not signed in.</p>
          )}
        </Card>

        <Card title="System-wide audio" icon={Sparkles}>
          <div className="flex flex-col gap-3">
            {systemStatus.supported && (
              <div className="flex items-center justify-between gap-3 rounded-control border border-accent/30 bg-accent-muted/40 px-3 py-2.5">
                <div className="min-w-0 text-sm">
                  <p className="font-medium text-accent-strong">
                    Equalize everything you hear
                  </p>
                  <p className="text-xs text-text-muted">
                    Routes all system audio through the equalizer and effects.
                  </p>
                  {runtimeStatus !== "disabled" && (
                    <span
                      className={`mt-1.5 inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium ${runtimeStatus === "recovering"
                          ? "bg-amber-500/15 text-amber-400"
                          : "bg-success/15 text-success"
                        }`}
                      title={
                        runtimeStatus === "recovering"
                          ? "A transient failure (e.g. heavy CPU load or a device change) is being recovered in the background — audio is restored but momentarily unequalised."
                          : "System-wide EQ is running."
                      }
                    >
                      <span
                        className={`size-1.5 rounded-full ${runtimeStatus === "recovering"
                            ? "animate-pulse bg-amber-400"
                            : "bg-success"
                          }`}
                        aria-hidden="true"
                      />
                      {runtimeStatus === "recovering" ? "Recovering…" : "Active"}
                    </span>
                  )}
                </div>
                <div className="flex shrink-0 gap-2">
                  {systemAffordance === "enable" ? (
                    <>
                      <Button variant="primary" onClick={startSystemAudio}>
                        {systemEqOn ? "Restart" : "Enable"}
                      </Button>
                      {systemEqOn && (
                        <Button variant="secondary" onClick={stopSystemEq}>
                          Stop
                        </Button>
                      )}
                    </>
                  ) : systemAffordance === "install" ? (
                    <Button
                      variant="primary"
                      onClick={() => void installAudioDriver()}
                      disabled={driverInstalling}
                    >
                      {driverInstalling ? "Installing…" : "Install audio driver"}
                    </Button>
                  ) : systemAffordance === "not-bundled" ? (
                    awaitingReboot ? (
                      <span className="text-xs text-text-muted">
                        Restart your PC to finish setup
                      </span>
                    ) : (
                      <Button
                        variant="primary"
                        onClick={() => void setupApo()}
                        disabled={setupPhase !== null}
                      >
                        {setupPhase === "installing"
                          ? "Installing…"
                          : "Set up system-wide EQ"}
                      </Button>
                    )
                  ) : (
                    <span className="text-xs text-text-muted">
                      Unavailable on this system
                    </span>
                  )}
                </div>
              </div>
            )}

            {systemStatus.supported && systemAffordance === "install" && (
              <p className="text-xs text-text-muted">
                System-wide EQ routes audio through the HypeMuzik virtual audio
                device. Installing it needs a one-time administrator approval.
              </p>
            )}
            {systemStatus.available && isMacos && (
              <div className="rounded-control border border-border bg-surface px-3 py-2.5">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-sm font-medium">Apply to</p>
                  <div className="flex gap-1 rounded-control bg-surface-overlay p-0.5">
                    {(
                      [
                        ["all", "All apps"],
                        ["only", "Only selected"],
                        ["except", "All except"],
                      ] as [SystemEqScopeMode, string][]
                    ).map(([mode, label]) => (
                      <button
                        key={mode}
                        type="button"
                        onClick={() => setScopeMode(mode)}
                        className={`rounded-[6px] px-2.5 py-1 text-xs transition-colors ${
                          systemEqScope.mode === mode
                            ? "bg-accent-muted text-accent-strong"
                            : "text-text-muted hover:text-text"
                        }`}
                        aria-pressed={systemEqScope.mode === mode}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                </div>

                {systemEqScope.mode !== "all" && (
                  <div className="mt-2.5 max-h-48 overflow-y-auto rounded-md border border-border">
                    {scopeApps.length === 0 ? (
                      <p className="px-3 py-3 text-xs text-text-faint">
                        No apps are playing audio right now. Start playback in another
                        app, then reopen this list.
                      </p>
                    ) : (
                      <ul className="divide-y divide-border/60">
                        {scopeApps.map((app) => {
                          const checked = systemEqScope.apps.includes(app.id);
                          return (
                            <li key={app.id}>
                              <button
                                type="button"
                                onClick={() => toggleScopeApp(app.id)}
                                className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface-overlay"
                                aria-pressed={checked}
                              >
                                <span
                                  className={`flex size-4 shrink-0 items-center justify-center rounded-[4px] border text-[10px] ${
                                    checked
                                      ? "border-accent bg-accent-muted text-accent-strong"
                                      : "border-border"
                                  }`}
                                  aria-hidden="true"
                                >
                                  {checked ? "✓" : ""}
                                </span>
                                {app.icon && (
                                  <img
                                    src={app.icon}
                                    alt=""
                                    className="size-4 shrink-0 rounded-[3px]"
                                  />
                                )}
                                <span className="min-w-0 flex-1 truncate text-text">
                                  {app.name}
                                </span>
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </div>
                )}
                <p className="mt-2 text-[11px] text-text-faint">
                  {systemEqScope.mode === "only"
                    ? "Only the checked apps are equalized."
                    : "Every app except the checked ones is equalized."}{" "}
                  Changes rebuild the tap; apps must be playing to appear.
                </p>
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

            {(captureError || systemEqError || driverError) && (
              <p className="text-sm text-danger">
                {captureError ?? systemEqError ?? driverError}
              </p>
            )}

            <div className="flex items-start gap-2 rounded-control border border-border bg-surface px-3 py-2 text-xs text-text-muted">
              <CircleAlert
                className="mt-0.5 size-3.5 shrink-0 text-text-faint"
                aria-hidden="true"
              />
              <span>
                {systemAffordance === "enable"
                  ? "Everything you hear is re-rendered through the chain. macOS taps other apps (first use prompts for audio-capture permission; the grant persists on a code-signed build); Linux routes through a PipeWire/PulseAudio virtual sink and restores your default output when stopped; Windows routes through the bundled HypeMuzik virtual audio device."
                  : systemAffordance === "install"
                    ? "System-wide equalization on Windows routes audio through the bundled HypeMuzik virtual audio device. Install the driver (one-time, admin-approved) to enable it."
                    : systemAffordance === "not-bundled"
                      ? "One-click setup installs VB-Audio's free VB-CABLE virtual audio device (downloaded from vb-audio.com, one admin approval) and routes all system audio through the equalizer. If the download fails, install VB-CABLE manually from vb-audio.com/Cable — it's detected automatically."
                      : "System-wide equalization isn't available here. See docs/system-eq.md."}
              </span>
            </div>
          </div>
        </Card>

        <VisualizerCard />
      </div>
    </div>
  );
}
