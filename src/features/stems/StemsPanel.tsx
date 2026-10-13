import { useCallback, useEffect, useRef, useState } from "react";
import { AudioLines, Download, Drum, Guitar, Loader2, Mic2 } from "lucide-react";
import { PremiumGate } from "@/components/PremiumGate";
import { Button } from "@/components/Button";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  onStemsDownload,
  onStemsError,
  onStemsProgress,
  stemsArm,
  stemsInstall,
  stemsInstallCancel,
  stemsReset,
  stemsSetGain,
  stemsStatus,
  type StemSource,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";
import {
  effectiveGains,
  initialControls,
  STEMS,
  type StemControls,
  type StemId,
} from "./mix";

const META: Record<
  StemId,
  { label: string; hint: string; icon: typeof Mic2; tone: string; fill: string }
> = {
  vocals: {
    label: "Vocals",
    hint: "Lead and backing voices",
    icon: Mic2,
    tone: "text-emerald-400",
    fill: "bg-emerald-500",
  },
  drums: {
    label: "Drums",
    hint: "Kick, snare, hats, percussion",
    icon: Drum,
    tone: "text-sky-400",
    fill: "bg-sky-500",
  },
  instruments: {
    label: "Instruments",
    hint: "Bass, keys, guitars and the rest",
    icon: Guitar,
    tone: "text-violet-400",
    fill: "bg-violet-500",
  },
};

const formatMb = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

/** Stem separation is a Supporter feature (it's real CPU work, not just a UI
 *  toggle) — gated at the whole-view level so a non-Supporter never even
 *  triggers the download or the separation pipeline below. */
export function StemsPanel() {
  return (
    <PremiumGate feature="STEMS" variant="page">
      <StemsPanelBody />
    </PremiumGate>
  );
}

/** Vocals / Drums / Instruments for whatever is playing (a local file or a
 *  YouTube Music track). The song separates in the background — the stretch
 *  around the playhead first — and each control goes live as its part is ready. */
function StemsPanelBody() {
  const trackPath = useEngineStore((s) => s.currentTrackPath);
  const ytVideoId = useEngineStore((s) => s.currentYtVideoId);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const source: StemSource | null = trackPath
    ? { kind: "file", path: trackPath }
    : ytVideoId
      ? { kind: "ytmusic", videoId: ytVideoId }
      : null;
  const sourceKey = source ? (source.kind === "file" ? `f:${source.path}` : `y:${source.videoId}`) : null;

  const [available, setAvailable] = useState(true);
  const [downloadBytes, setDownloadBytes] = useState(0);
  const [downloading, setDownloading] = useState(false);
  const [downloadFrac, setDownloadFrac] = useState(0);
  const [armed, setArmed] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [controls, setControls] = useState<StemControls>(initialControls);
  // Bumped to re-run the arm effect after the install finishes.
  const [attempt, setAttempt] = useState(0);
  const sourceRef = useRef(source);
  sourceRef.current = source;

  const push = useCallback((c: StemControls) => {
    effectiveGains(c).forEach((g, i) => void stemsSetGain(i, g).catch(() => {}));
  }, []);

  const update = (id: StemId, patch: Partial<StemControls[StemId]>) => {
    const next = { ...controls, [id]: { ...controls[id], ...patch } };
    setControls(next);
    push(next);
  };

  useEffect(() => {
    const offs: Array<() => void> = [];
    let dead = false;
    const keep = (p: Promise<() => void>) =>
      p.then((fn) => (dead ? fn() : offs.push(fn))).catch(() => {});
    keep(onStemsProgress((v) => setProgress(v)));
    keep(onStemsDownload((p) => setDownloadFrac(p.total ? p.done / p.total : 0)));
    keep(onStemsError((m) => setError(m)));
    return () => {
      dead = true;
      offs.forEach((fn) => fn());
    };
  }, []);

  // Arm the playing track (no button) and re-arm when it changes.
  useEffect(() => {
    if (!sourceKey || !sourceRef.current) {
      setArmed(false);
      return;
    }
    const src = sourceRef.current;
    let cancelled = false;
    void (async () => {
      setError(null);
      setArmed(false);
      try {
        const status = await stemsStatus(src);
        if (cancelled) return;
        setAvailable(status.available);
        setDownloadBytes(status.downloadBytes);
        if (!status.available) return;

        setPreparing(true);
        setProgress(status.separated ? 1 : 0);
        const fresh = initialControls();
        setControls(fresh);
        await stemsArm(src);
        if (cancelled) return;
        push(fresh);
        setArmed(true);
      } catch (e) {
        if (!cancelled) setError(ipcErrorMessage(e));
      } finally {
        if (!cancelled) setPreparing(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [sourceKey, attempt, push]);

  // Leaving the Stems view: back to the original mix, and stop separating.
  useEffect(() => {
    return () => {
      void stemsReset().catch(() => {});
    };
  }, []);

  const install = async () => {
    setError(null);
    setDownloading(true);
    setDownloadFrac(0);
    try {
      await stemsInstall();
      setAttempt((n) => n + 1);
      const status = await stemsStatus(sourceRef.current);
      setAvailable(status.available);
      setDownloadBytes(status.downloadBytes);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setDownloading(false);
    }
  };

  if (!available) {
    return (
      <Shell>
        <div className="flex flex-col items-center gap-4 py-8 text-center">
          <AudioLines className="size-8 text-text-faint" aria-hidden="true" />
          <div className="max-w-sm space-y-1">
            <p className="text-sm font-medium">One-time download</p>
            <p className="text-sm text-text-muted">
              Stems run on your computer with Deezer&rsquo;s open-source Spleeter models
              (MIT licence). {formatMb(downloadBytes)}, checked against a fingerprint
              before use.
            </p>
          </div>
          {downloading ? (
            <div className="w-full max-w-xs">
              <div className="h-2 overflow-hidden rounded-full bg-surface">
                <div
                  className="h-full rounded-full bg-accent transition-[width] duration-200"
                  style={{ width: `${Math.round(downloadFrac * 100)}%` }}
                />
              </div>
              <div className="mt-2 flex items-center justify-between text-xs text-text-faint">
                <span>{Math.round(downloadFrac * 100)}%</span>
                <button type="button" className="hover:text-text" onClick={() => void stemsInstallCancel()}>
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <Button variant="primary" onClick={() => void install()}>
              <Download className="size-4" /> Download stems
            </Button>
          )}
          {error && <p className="text-sm text-danger">{error}</p>}
        </div>
      </Shell>
    );
  }

  if (!source) {
    return (
      <Shell>
        <div className="flex flex-col items-center gap-3 py-10 text-center">
          <AudioLines className="size-8 text-text-faint" aria-hidden="true" />
          <p className="max-w-sm text-sm text-text-muted">
            Play a track from your library or YouTube Music — you can then mix its vocals,
            drums and instruments live.
          </p>
        </div>
      </Shell>
    );
  }

  const filling = armed && progress < 1;
  return (
    <Shell>
      <div className="mb-4">
        <p className="truncate text-sm font-medium">{nowPlaying ?? "—"}</p>
        <p className="text-xs text-text-faint" aria-live="polite">
          {preparing
            ? "Getting the stems ready — the track keeps playing…"
            : filling
              ? `Separating the rest of the song… ${Math.round(progress * 100)}%`
              : armed
                ? "Drag a fader, mute, or solo a part"
                : "Ready"}
        </p>
      </div>

      {preparing && (
        <div className="mb-4 flex items-center gap-2 text-sm text-text-muted">
          <Loader2 className="size-4 animate-spin" /> Separating around the playhead…
        </div>
      )}
      {error && <p className="mb-4 text-sm text-danger">{error}</p>}

      <ul className={cn("space-y-3 transition-opacity", armed ? "opacity-100" : "pointer-events-none opacity-50")}>
        {STEMS.map((id) => {
          const m = META[id];
          const c = controls[id];
          const Icon = m.icon;
          const silent = effectiveGains(controls)[STEMS.indexOf(id)] === 0;
          return (
            <li key={id} className="rounded-card border border-border bg-surface p-3">
              <div className="flex items-center gap-3">
                <Icon className={cn("size-5 shrink-0", m.tone, silent && "opacity-40")} aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-semibold">{m.label}</p>
                  <p className="truncate text-xs text-text-faint">{m.hint}</p>
                </div>
                <ToggleChip
                  label={`Mute ${m.label}`}
                  text="M"
                  on={c.muted}
                  onClass="bg-danger/90 text-white border-danger"
                  onClick={() => update(id, { muted: !c.muted })}
                />
                <ToggleChip
                  label={`Solo ${m.label}`}
                  text="S"
                  on={c.solo}
                  onClass={cn(m.fill, "text-white border-transparent")}
                  onClick={() => update(id, { solo: !c.solo })}
                />
              </div>
              <div className="mt-3 flex items-center gap-3">
                <Slider
                  className="flex-1"
                  label={`${m.label} level`}
                  value={c.level}
                  min={0}
                  max={1.5}
                  step={0.01}
                  disabled={!armed}
                  onChange={(v) => update(id, { level: v })}
                  formatValue={(v) => `${Math.round(v * 100)}%`}
                />
                <span className="w-10 text-right text-xs tabular-nums text-text-muted">
                  {Math.round(c.level * 100)}%
                </span>
              </div>
            </li>
          );
        })}
      </ul>

      <p className="mt-4 text-center text-[11px] text-text-faint">
        At 100% everywhere you hear the original track, unchanged.
      </p>
    </Shell>
  );
}

function ToggleChip({
  label,
  text,
  on,
  onClass,
  onClick,
}: {
  label: string;
  text: string;
  on: boolean;
  onClass: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={on}
      title={label}
      onClick={onClick}
      className={cn(
        "flex size-8 items-center justify-center rounded-control border text-xs font-bold transition-colors",
        on ? onClass : "border-border text-text-muted hover:bg-surface-raised hover:text-text",
      )}
    >
      {text}
    </button>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="mx-auto max-w-2xl">
      <div className="mb-4">
        <h1 className="text-xl font-semibold">Stems</h1>
        <p className="text-sm text-text-muted">
          Mix the vocals, drums and instruments of the playing track — live.
        </p>
      </div>
      <div className="rounded-card border border-border bg-surface-raised p-5">{children}</div>
    </div>
  );
}
