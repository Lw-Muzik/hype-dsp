import { useCallback, useEffect, useRef, useState } from "react";
import { AudioLines, Cpu, Loader2, Sparkles } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  onStemsProgress,
  stemsArm,
  stemsReset,
  stemsSetGain,
  stemsStatus,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";

// Mix elements (must match hm-audio::stems): drums are split into kick + hihat.
const EL = { vocals: 0, kick: 1, hihat: 2, bass: 3, melody: 4 } as const;
const ELEMENT_COUNT = 5;

type PadKind = "mute" | "solo";
interface Pad {
  id: string;
  label: string;
  kind: PadKind;
  group: number[];
  /** [engaged classes, idle classes] */
  engaged: string;
  idle: string;
  combo?: boolean;
}

// Order = VirtualDJ's grid: row 1 then row 2.
const PADS: Pad[] = [
  {
    id: "vocal", label: "Vocal", kind: "mute", group: [EL.vocals],
    engaged: "bg-emerald-500 text-white border-emerald-500",
    idle: "border-emerald-500/40 text-emerald-400 hover:bg-emerald-500/10",
  },
  {
    id: "instru", label: "Instru", kind: "mute", group: [EL.kick, EL.hihat, EL.bass, EL.melody],
    engaged: "bg-amber-500 text-white border-amber-500",
    idle: "border-amber-500/40 text-amber-400 hover:bg-amber-500/10",
  },
  {
    id: "bass", label: "Bass", kind: "mute", group: [EL.bass],
    engaged: "bg-rose-500 text-white border-rose-500",
    idle: "border-rose-500/40 text-rose-400 hover:bg-rose-500/10",
  },
  {
    id: "acapella", label: "Acapella", kind: "solo", group: [EL.vocals], combo: true,
    engaged: "bg-emerald-500/90 text-white border-emerald-400",
    idle: "border-border text-text-faint hover:bg-surface-raised",
  },
  {
    id: "kick", label: "Kick", kind: "mute", group: [EL.kick],
    engaged: "bg-sky-500 text-white border-sky-500",
    idle: "border-sky-500/40 text-sky-400 hover:bg-sky-500/10",
  },
  {
    id: "hihat", label: "HiHat", kind: "mute", group: [EL.hihat],
    engaged: "bg-cyan-500 text-white border-cyan-500",
    idle: "border-cyan-500/40 text-cyan-400 hover:bg-cyan-500/10",
  },
  {
    id: "melody", label: "FX: Melody", kind: "mute", group: [EL.melody],
    engaged: "bg-violet-500 text-white border-violet-500",
    idle: "border-violet-500/40 text-violet-400 hover:bg-violet-500/10",
  },
  {
    id: "instrument", label: "Instrument", kind: "solo", group: [EL.kick, EL.hihat, EL.bass, EL.melody], combo: true,
    engaged: "bg-amber-500/90 text-white border-amber-400",
    idle: "border-border text-text-faint hover:bg-surface-raised",
  },
];

const sameSet = (a: number[], b: number[]) =>
  a.length === b.length && a.every((x) => b.includes(x));

/** VirtualDJ-style stems pad grid: the playing track auto-separates in the
 *  background (htdemucs on CoreML, drums split into kick/hihat), then the pads
 *  go live — tap to mute a stem, tap a combo pad to isolate. No "separate" step. */
export function StemsPanel() {
  const trackPath = useEngineStore((s) => s.currentTrackPath);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [available, setAvailable] = useState(true);
  const [accelerated, setAccelerated] = useState(false);
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const [muted, setMuted] = useState<boolean[]>(() => Array(ELEMENT_COUNT).fill(false));
  const [solo, setSolo] = useState<number[] | null>(null);
  const armedFor = useRef<string | null>(null);

  /** Push effective gains (mute = 0; solo isolates its group) to all elements. */
  const applyAll = useCallback((m: boolean[], s: number[] | null) => {
    for (let e = 0; e < ELEMENT_COUNT; e++) {
      const gain = s !== null ? (s.includes(e) ? 1 : 0) : (m[e] ? 0 : 1);
      void stemsSetGain(e, gain).catch(() => {});
    }
  }, []);

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
        const m = Array(ELEMENT_COUNT).fill(false);
        setMuted(m);
        setSolo(null);
        applyAll(m, null);
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

  // Leaving the Stems view: restore the original mix (all elements to unity).
  useEffect(() => {
    return () => {
      void stemsReset().catch(() => {});
    };
  }, []);

  const tapPad = (pad: Pad) => {
    if (!armed) return;
    if (pad.kind === "mute") {
      const m = muted.slice();
      const allMuted = pad.group.every((e) => m[e]);
      pad.group.forEach((e) => (m[e] = !allMuted));
      setMuted(m);
      setSolo(null); // mute and solo are distinct modes
      applyAll(m, null);
    } else {
      const active = solo !== null && sameSet(solo, pad.group);
      const s = active ? null : pad.group;
      const m = Array(ELEMENT_COUNT).fill(false);
      setMuted(m);
      setSolo(s);
      applyAll(m, s);
    }
  };

  const isEngaged = (pad: Pad) =>
    pad.kind === "mute"
      ? pad.group.every((e) => muted[e])
      : solo !== null && sameSet(solo, pad.group);

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

  return (
    <Shell accelerated={accelerated}>
      <div className="mb-4">
        <p className="truncate text-sm font-medium">{nowPlaying ?? "—"}</p>
        <p className="text-xs text-text-faint">
          {busy
            ? "Preparing stems — the track keeps playing…"
            : armed
              ? "Tap a pad to mute a stem · tap a combo pad to isolate"
              : "Ready"}
        </p>
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
          "grid grid-cols-4 gap-2 transition-opacity",
          armed ? "opacity-100" : "pointer-events-none opacity-50",
        )}
      >
        {PADS.map((pad) => (
          <button
            key={pad.id}
            type="button"
            disabled={!armed}
            onClick={() => tapPad(pad)}
            className={cn(
              "flex h-16 items-center justify-center rounded-card border-2 px-2 text-sm font-semibold transition-colors",
              pad.combo && "italic",
              isEngaged(pad) ? pad.engaged : pad.idle,
            )}
          >
            {pad.combo ? `(${pad.label})` : pad.label}
          </button>
        ))}
      </div>

      <p className="mt-4 text-center text-[11px] text-text-faint">
        Kick / HiHat are a crossover split of the isolated drum stem.
      </p>
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
            Isolate or drop vocals, bass, kick, hihat and melody from the playing track — live.
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

function Empty({ icon: Icon, text }: { icon: typeof AudioLines; text: string }) {
  return (
    <div className="flex flex-col items-center gap-3 py-10 text-center">
      <Icon className="size-8 text-text-faint" aria-hidden="true" />
      <p className="max-w-sm text-sm text-text-muted">{text}</p>
    </div>
  );
}
