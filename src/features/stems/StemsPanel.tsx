import { useCallback, useEffect, useRef, useState } from "react";
import {
  AudioLines,
  Cpu,
  Disc3,
  Drum,
  Guitar,
  Loader2,
  Mic2,
  Music2,
  Sparkles,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  onStemsProgress,
  stemsArm,
  stemsReset,
  stemsSetGain,
  stemsStatus,
  type StemMode,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";

/** A fader. In 4-stem mode each controls one slot; in 2-stem mode the
 *  Instrumental fader controls drums+bass+other (slots 1,2,3) together. */
interface StemControl {
  id: string;
  label: string;
  icon: LucideIcon;
  color: string;
  slots: number[];
}

const LAYOUTS: Record<StemMode, StemControl[]> = {
  four: [
    { id: "vocals", label: "Vocals", icon: Mic2, color: "bg-accent", slots: [0] },
    { id: "drums", label: "Drums", icon: Drum, color: "bg-danger", slots: [1] },
    { id: "bass", label: "Bass", icon: Music2, color: "bg-sky-500", slots: [2] },
    { id: "other", label: "Instruments", icon: Guitar, color: "bg-success", slots: [3] },
  ],
  two: [
    { id: "vocals", label: "Vocals", icon: Mic2, color: "bg-accent", slots: [0] },
    { id: "inst", label: "Instrumental", icon: Disc3, color: "bg-success", slots: [1, 2, 3] },
  ],
};

const MODES: { id: StemMode; label: string }[] = [
  { id: "two", label: "Vocals · Instrumental" },
  { id: "four", label: "4 stems" },
];

const STEM_COUNT = 4;
const sameSet = (a: number[], b: number[]) =>
  a.length === b.length && a.every((x) => b.includes(x));

/** VirtualDJ-style stems: the current track auto-separates in the background
 *  (htdemucs on CoreML) while it keeps playing, then the faders go live so you
 *  can tap a stem to isolate/mute it instantly — no "separate" step. */
export function StemsPanel() {
  const trackPath = useEngineStore((s) => s.currentTrackPath);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [mode, setMode] = useState<StemMode>("four");
  const [available, setAvailable] = useState(true);
  const [accelerated, setAccelerated] = useState(false);
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  // Per-slot mix state (the 2-stem layout writes/reads slot groups).
  const [gains, setGains] = useState<number[]>([1, 1, 1, 1]);
  const [muted, setMuted] = useState<boolean[]>([false, false, false, false]);
  const [solo, setSolo] = useState<number[] | null>(null);
  const armedFor = useRef<string | null>(null);

  /** Push effective gains (factoring mute + solo) to all four slots. */
  const applyAll = useCallback(
    (g: number[], m: boolean[], s: number[] | null) => {
      for (let i = 0; i < STEM_COUNT; i++) {
        const gi = g[i] ?? 1;
        const effective = s !== null ? (s.includes(i) ? gi : 0) : (m[i] ?? false) ? 0 : gi;
        void stemsSetGain(i, effective).catch(() => {});
      }
    },
    [],
  );

  // Live separation progress (0..1).
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    onStemsProgress((v) => setProgress(v))
      .then((fn) => (cancelled ? fn() : (un = fn)))
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  // Auto-arm the current track (no button) and re-arm when it changes.
  useEffect(() => {
    if (!trackPath) {
      setArmed(false);
      return;
    }
    let cancelled = false;
    void (async () => {
      setError(null);
      setArmed(false);
      try {
        const status = await stemsStatus(trackPath);
        if (cancelled) return;
        setAvailable(status.available);
        setAccelerated(status.accelerated);
        if (!status.available) return;

        setBusy(true);
        setProgress(status.separated ? 1 : 0);
        await stemsArm(trackPath);
        if (cancelled) return;

        armedFor.current = trackPath;
        const g = [1, 1, 1, 1];
        const m = [false, false, false, false];
        setGains(g);
        setMuted(m);
        setSolo(null);
        applyAll(g, m, null);
        setArmed(true);
      } catch (e) {
        if (!cancelled) setError(ipcErrorMessage(e));
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [trackPath, applyAll]);

  // Leaving the Stems view: restore the original mix (all stems to unity).
  useEffect(() => {
    return () => {
      void stemsReset().catch(() => {});
    };
  }, []);

  const setControlGain = (slots: number[], v: number) => {
    const g = gains.slice();
    slots.forEach((i) => (g[i] = v));
    setGains(g);
    applyAll(g, muted, solo);
  };
  const toggleMute = (slots: number[]) => {
    const next = !(muted[slots[0]!] ?? false);
    const m = muted.slice();
    slots.forEach((i) => (m[i] = next));
    setMuted(m);
    applyAll(gains, m, solo);
  };
  const toggleSolo = (slots: number[]) => {
    const s = solo !== null && sameSet(solo, slots) ? null : slots;
    setSolo(s);
    applyAll(gains, muted, s);
  };

  if (!trackPath) {
    return (
      <Shell accelerated={accelerated}>
        <Empty icon={AudioLines} text="Play a local track — its stems will be ready to mix in seconds." />
      </Shell>
    );
  }
  if (!available) {
    return (
      <Shell accelerated={accelerated}>
        <Empty
          icon={AudioLines}
          text="The stem separator isn’t installed yet. Run scripts/get_stems_model.sh to fetch htdemucs + ONNX Runtime, then restart."
        />
      </Shell>
    );
  }

  const layout = LAYOUTS[mode];

  return (
    <Shell accelerated={accelerated}>
      <div className="mb-4 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{nowPlaying ?? "—"}</p>
          <p className="text-xs text-text-faint">
            {busy
              ? "Preparing stems — the track keeps playing…"
              : armed
                ? "Tap a stem to isolate or mute it — live"
                : "Ready"}
          </p>
        </div>
        {/* Mode is a free, instant regrouping of the same separation. */}
        <div className="inline-flex shrink-0 rounded-control border border-border bg-surface p-0.5">
          {MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              onClick={() => setMode(m.id)}
              className={cn(
                "rounded-[8px] px-3 py-1.5 text-xs font-medium transition-colors",
                mode === m.id ? "bg-accent text-surface" : "text-text-muted hover:text-text",
              )}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>

      {busy && (
        <div className="mb-5">
          <div className="mb-2 flex items-center gap-2 text-sm text-text-muted">
            <Loader2 className="size-4 animate-spin" /> Separating…
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-surface">
            <div
              className="h-full rounded-full bg-accent transition-[width] duration-200"
              style={{ width: `${Math.round(progress * 100)}%` }}
            />
          </div>
          <p className="mt-1 text-center text-xs text-text-faint">
            {Math.round(progress * 100)}% — cached after the first time, then instant.
          </p>
        </div>
      )}

      {error && <p className="mb-4 text-sm text-danger">{error}</p>}

      <div
        className={cn(
          "grid gap-3 transition-opacity",
          mode === "two" ? "grid-cols-2" : "grid-cols-4",
          armed ? "opacity-100" : "pointer-events-none opacity-50",
        )}
      >
        {layout.map((ctrl) => {
          const gain = gains[ctrl.slots[0]!] ?? 1;
          const isMuted = muted[ctrl.slots[0]!] ?? false;
          const isSolo = solo !== null && sameSet(solo, ctrl.slots);
          const dimmed = isMuted || (solo !== null && !isSolo);
          const Icon = ctrl.icon;
          return (
            <div
              key={ctrl.id}
              className="flex flex-col items-center gap-3 rounded-card border border-border bg-surface p-3"
            >
              <div className="flex items-center gap-1.5 text-sm font-medium">
                <Icon className="size-4 text-text-muted" aria-hidden="true" />
                {ctrl.label}
              </div>
              <div className="flex h-44 items-end">
                <input
                  type="range"
                  min={0}
                  max={1.5}
                  step={0.01}
                  value={gain}
                  disabled={!armed}
                  onChange={(e) => setControlGain(ctrl.slots, Number(e.target.value))}
                  aria-label={`${ctrl.label} volume`}
                  className="h-44 accent-accent"
                  style={{ writingMode: "vertical-lr", direction: "rtl" }}
                />
              </div>
              <span className="text-xs tabular-nums text-text-faint">
                {Math.round(gain * 100)}%
              </span>
              <div className="flex w-full gap-1.5">
                <button
                  type="button"
                  disabled={!armed}
                  onClick={() => toggleMute(ctrl.slots)}
                  className={cn(
                    "flex-1 rounded-control py-1 text-xs font-semibold transition-colors disabled:opacity-50",
                    isMuted
                      ? "bg-danger/20 text-danger"
                      : "bg-surface-raised text-text-muted hover:text-text",
                  )}
                >
                  {isMuted ? "Muted" : "Mute"}
                </button>
                <button
                  type="button"
                  disabled={!armed}
                  onClick={() => toggleSolo(ctrl.slots)}
                  className={cn(
                    "flex-1 rounded-control py-1 text-xs font-semibold transition-colors disabled:opacity-50",
                    isSolo
                      ? "bg-accent text-surface"
                      : "bg-surface-raised text-text-muted hover:text-text",
                  )}
                >
                  Solo
                </button>
              </div>
              <div
                className={cn(
                  "h-1 w-full rounded-full transition-opacity",
                  ctrl.color,
                  dimmed ? "opacity-20" : "opacity-100",
                )}
              />
            </div>
          );
        })}
      </div>
    </Shell>
  );
}

function Shell({
  children,
  accelerated,
}: {
  children: React.ReactNode;
  accelerated: boolean;
}) {
  return (
    <div className="mx-auto max-w-2xl">
      <div className="mb-4 flex items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">Stems</h1>
          <p className="text-sm text-text-muted">
            Split the playing track into vocals, drums, bass and instruments — mix them live.
          </p>
        </div>
        <span
          className={cn(
            "mt-1 inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium",
            accelerated ? "bg-accent/15 text-accent" : "bg-surface text-text-faint",
          )}
          title={
            accelerated
              ? "Separating on the Apple Neural Engine / GPU"
              : "Separating on CPU (no CoreML)"
          }
        >
          {accelerated ? <Sparkles className="size-3" /> : <Cpu className="size-3" />}
          {accelerated ? "Neural Engine" : "CPU"}
        </span>
      </div>
      <div className="rounded-card border border-border bg-surface-raised p-5">{children}</div>
    </div>
  );
}

function Empty({ icon: Icon, text }: { icon: LucideIcon; text: string }) {
  return (
    <div className="flex flex-col items-center gap-3 py-10 text-center">
      <Icon className="size-8 text-text-faint" aria-hidden="true" />
      <p className="max-w-sm text-sm text-text-muted">{text}</p>
    </div>
  );
}
