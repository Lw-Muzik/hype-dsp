import { useCallback, useEffect, useState } from "react";
import {
  AudioLines,
  Disc3,
  Drum,
  Guitar,
  Loader2,
  Mic2,
  Music2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  onStemsProgress,
  stemsSeparate,
  stemsSetGain,
  stemsStatus,
  type StemMode,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";

interface StemDef {
  slot: number; // playback slot (matches the backend)
  label: string;
  icon: LucideIcon;
  color: string;
}

// Which faders each mode shows, and the slot each controls.
const LAYOUTS: Record<StemMode, StemDef[]> = {
  four: [
    { slot: 0, label: "Vocals", icon: Mic2, color: "bg-accent" },
    { slot: 1, label: "Drums", icon: Drum, color: "bg-danger" },
    { slot: 2, label: "Bass", icon: Music2, color: "bg-sky-500" },
    { slot: 3, label: "Instruments", icon: Guitar, color: "bg-success" },
  ],
  two: [
    { slot: 0, label: "Vocals", icon: Mic2, color: "bg-accent" },
    { slot: 1, label: "Instrumental", icon: Disc3, color: "bg-success" },
  ],
};

const MODES: { id: StemMode; label: string; hint: string }[] = [
  { id: "two", label: "Vocals + Instrumental", hint: "faster" },
  { id: "four", label: "Full (4 stems)", hint: "best" },
];

/** VirtualDJ-style stem separation: split the track, then mix the stems live
 *  with vertical faders + mute/solo. 2-stem (quick) or 4-stem. */
export function StemsPanel() {
  const trackPath = useEngineStore((s) => s.currentTrackPath);
  const nowPlaying = useEngineStore((s) => s.nowPlaying);

  const [mode, setMode] = useState<StemMode>("four");
  const [available, setAvailable] = useState(true);
  const [separated, setSeparated] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const [gains, setGains] = useState<number[]>([1, 1, 1, 1]);
  const [muted, setMuted] = useState<boolean[]>([false, false, false, false]);
  const [solo, setSolo] = useState<number | null>(null);

  // Refresh availability + cached state when the track or mode changes.
  useEffect(() => {
    setSeparated(false);
    if (!trackPath) return;
    let active = true;
    void stemsStatus(trackPath, mode)
      .then((s) => {
        if (!active) return;
        setAvailable(s.available);
        setSeparated(s.separated);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [trackPath, mode]);

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

  /** Push effective gains (factoring mute + solo) for all four slots. */
  const apply = useCallback((g: number[], m: boolean[], s: number | null) => {
    for (let i = 0; i < 4; i++) {
      const gi = g[i] ?? 1;
      const isMuted = m[i] ?? false;
      const effective = s !== null ? (i === s ? gi : 0) : isMuted ? 0 : gi;
      void stemsSetGain(i, effective).catch(() => {});
    }
  }, []);

  const separate = async () => {
    if (!trackPath) return;
    setBusy(true);
    setError(null);
    setProgress(0);
    try {
      await stemsSeparate(trackPath, mode);
      setSeparated(true);
      const g = [1, 1, 1, 1];
      const m = [false, false, false, false];
      setGains(g);
      setMuted(m);
      setSolo(null);
      apply(g, m, null);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const setGain = (slot: number, v: number) => {
    const g = gains.slice();
    g[slot] = v;
    setGains(g);
    apply(g, muted, solo);
  };
  const toggleMute = (slot: number) => {
    const m = muted.slice();
    m[slot] = !m[slot];
    setMuted(m);
    apply(gains, m, solo);
  };
  const toggleSolo = (slot: number) => {
    const s = solo === slot ? null : slot;
    setSolo(s);
    apply(gains, muted, s);
  };

  if (!trackPath) {
    return (
      <Shell>
        <Empty icon={AudioLines} text="Play a local track to separate it into stems." />
      </Shell>
    );
  }
  if (!available) {
    return (
      <Shell>
        <Empty
          icon={AudioLines}
          text="The stem separator isn’t installed yet. Build it with scripts/build_demucs.sh, then restart."
        />
      </Shell>
    );
  }

  const layout = LAYOUTS[mode];

  return (
    <Shell>
      <div className="mb-4 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{nowPlaying ?? "—"}</p>
          <p className="text-xs text-text-faint">
            {separated ? "Separated — mix the stems below" : "Ready to separate"}
          </p>
        </div>
        {!separated && !busy && (
          <Button variant="primary" onClick={() => void separate()}>
            <AudioLines className="size-4" /> Separate
          </Button>
        )}
      </div>

      {/* Mode picker (locked while separating). */}
      {!separated && (
        <div className="mb-5 inline-flex rounded-control border border-border bg-surface p-0.5">
          {MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              disabled={busy}
              onClick={() => setMode(m.id)}
              className={cn(
                "rounded-[8px] px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50",
                mode === m.id
                  ? "bg-accent text-surface"
                  : "text-text-muted hover:text-text",
              )}
            >
              {m.label}
              <span className="ml-1 opacity-60">· {m.hint}</span>
            </button>
          ))}
        </div>
      )}

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
            {Math.round(progress * 100)}% — this runs once, then it’s cached.
          </p>
        </div>
      )}

      {error && <p className="mb-4 text-sm text-danger">{error}</p>}

      {separated && (
        <div
          className={cn(
            "grid gap-3",
            mode === "two" ? "grid-cols-2" : "grid-cols-4",
          )}
        >
          {layout.map((stem) => {
            const gain = gains[stem.slot] ?? 1;
            const isMutedFlag = muted[stem.slot] ?? false;
            const isSolo = solo === stem.slot;
            const dimmed = isMutedFlag || (solo !== null && !isSolo);
            const Icon = stem.icon;
            return (
              <div
                key={stem.slot}
                className="flex flex-col items-center gap-3 rounded-card border border-border bg-surface p-3"
              >
                <div className="flex items-center gap-1.5 text-sm font-medium">
                  <Icon className="size-4 text-text-muted" aria-hidden="true" />
                  {stem.label}
                </div>
                <div className="flex h-44 items-end">
                  <input
                    type="range"
                    min={0}
                    max={1.5}
                    step={0.01}
                    value={gain}
                    onChange={(e) => setGain(stem.slot, Number(e.target.value))}
                    aria-label={`${stem.label} volume`}
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
                    onClick={() => toggleMute(stem.slot)}
                    className={cn(
                      "flex-1 rounded-control py-1 text-xs font-semibold transition-colors",
                      isMutedFlag
                        ? "bg-danger/20 text-danger"
                        : "bg-surface-raised text-text-muted hover:text-text",
                    )}
                  >
                    {isMutedFlag ? "Muted" : "Mute"}
                  </button>
                  <button
                    type="button"
                    onClick={() => toggleSolo(stem.slot)}
                    className={cn(
                      "flex-1 rounded-control py-1 text-xs font-semibold transition-colors",
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
                    stem.color,
                    dimmed ? "opacity-20" : "opacity-100",
                  )}
                />
              </div>
            );
          })}
        </div>
      )}

      {separated && (
        <button
          type="button"
          onClick={() => setSeparated(false)}
          className="mt-4 text-xs text-text-muted transition-colors hover:text-text"
        >
          ← Re-separate or switch mode
        </button>
      )}
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="mx-auto max-w-2xl">
      <div className="mb-4">
        <h1 className="text-xl font-semibold">Stems</h1>
        <p className="text-sm text-text-muted">
          Split the track into vocals, drums, bass and instruments — then remix.
        </p>
      </div>
      <div className="rounded-card border border-border bg-surface-raised p-5">
        {children}
      </div>
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
