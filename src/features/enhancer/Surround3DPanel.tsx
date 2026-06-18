import { useEffect } from "react";
import { Speaker, Waves } from "lucide-react";
import { Slider } from "@/components/Slider";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import type { SurroundSpeakers } from "@/lib/types";
import { cn } from "@/lib/cn";

/** A satellite speaker in the ring: its toggle key, label, and azimuth. */
interface Satellite {
  key: keyof SurroundSpeakers;
  label: string;
  /** Degrees clockwise from front-centre (negative = left). */
  azimuth: number;
}

const SATELLITES: readonly Satellite[] = [
  { key: "frontL", label: "Left Front", azimuth: -30 },
  { key: "frontR", label: "Right Front", azimuth: 30 },
  { key: "sideL", label: "Tweeter", azimuth: -90 },
  { key: "sideR", label: "Tweeter", azimuth: 90 },
  { key: "surroundL", label: "Left Surround", azimuth: -135 },
  { key: "surroundR", label: "Right Surround", azimuth: 135 },
];

/** Radius of the speaker ring as a percentage of the (square) stage half-size. */
const RING_RADIUS = 40;

/** Place a node at `azimuth` on the ring; returns CSS left/top percentages. */
function ringPosition(azimuth: number): { left: string; top: string } {
  const rad = (azimuth * Math.PI) / 180;
  return {
    left: `${50 + RING_RADIUS * Math.sin(rad)}%`,
    top: `${50 - RING_RADIUS * Math.cos(rad)}%`,
  };
}

interface Surround3DPanelProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Full-screen "3D Surround" configurator: a radial ring of virtual speakers
 * around the listener, a global intensity, per-speaker toggles, and a subwoofer
 * level — mirroring `Surround3DState` and pushing every change to the engine.
 */
export function Surround3DPanel({ open, onClose }: Surround3DPanelProps) {
  const surround = useEngineStore((s) => s.state.surround3d);
  const setSurround3d = useEngineStore((s) => s.setSurround3d);
  const { intensity, subwoofer, speakers } = surround;

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const toggleSpeaker = (key: keyof SurroundSpeakers) =>
    setSurround3d({
      ...surround,
      enabled: true,
      speakers: { ...speakers, [key]: !speakers[key] },
    });

  const subActive = subwoofer > 0;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-label="3D Surround"
    >
      <div className="absolute inset-0 bg-black/70" onClick={onClose} aria-hidden="true" />

      <div className="relative z-10 flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden rounded-card border border-border bg-[#08080c] shadow-2xl">
        {/* Header: title + global intensity */}
        <div className="flex items-center justify-between gap-6 px-6 pt-5">
          <h2 className="text-lg font-semibold tracking-tight">3D Surround</h2>
          <div className="flex items-center gap-3">
            <span className="text-xs font-medium uppercase tracking-wider text-text-muted">
              Intensity
            </span>
            <Slider
              label="Surround intensity"
              min={0}
              max={1}
              step={0.01}
              value={intensity}
              onChange={(v) => setSurround3d({ ...surround, enabled: true, intensity: v })}
              formatValue={(v) => `${Math.round(v * 100)} percent`}
              className="w-40"
            />
            <span className="w-10 text-right text-xs tabular-nums text-text-muted">
              {Math.round(intensity * 100)}%
            </span>
          </div>
        </div>

        {/* Radial speaker stage */}
        <div className="px-6 py-2">
          <div className="relative mx-auto aspect-square w-full max-w-xl">
            {/* Concentric range rings + connector lines */}
            <svg
              className="absolute inset-0 h-full w-full"
              viewBox="0 0 100 100"
              aria-hidden="true"
            >
              {[14, 22, 30, 38, 46].map((r) => (
                <circle
                  key={r}
                  cx="50"
                  cy="50"
                  r={r}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="0.2"
                  className="text-white/10"
                />
              ))}
              {SATELLITES.map((s) => {
                const rad = (s.azimuth * Math.PI) / 180;
                const x = 50 + RING_RADIUS * Math.sin(rad);
                const y = 50 - RING_RADIUS * Math.cos(rad);
                return (
                  <line
                    key={s.key}
                    x1="50"
                    y1="50"
                    x2={x}
                    y2={y}
                    stroke="currentColor"
                    strokeWidth="0.3"
                    strokeDasharray="1 1.5"
                    className={speakers[s.key] ? "text-accent/60" : "text-white/12"}
                  />
                );
              })}
              {/* Subwoofer connector (front-centre down to the sub) */}
              <line
                x1="50"
                y1="50"
                x2="50"
                y2={50 + RING_RADIUS}
                stroke="currentColor"
                strokeWidth="0.3"
                strokeDasharray="1 1.5"
                className={subActive ? "text-accent/60" : "text-white/12"}
              />
            </svg>

            {/* Listener head (top-down) at centre */}
            <div
              className="absolute -translate-x-1/2 -translate-y-1/2"
              style={{ left: "50%", top: "50%" }}
              aria-hidden="true"
            >
              <div className="relative grid size-16 place-items-center rounded-full bg-gradient-to-b from-slate-500 to-slate-700 shadow-lg">
                {/* nose (front marker) */}
                <span className="absolute -top-1 size-3 rounded-full bg-slate-400" />
                {/* ears */}
                <span className="absolute -left-1 size-2.5 rounded-full bg-slate-600" />
                <span className="absolute -right-1 size-2.5 rounded-full bg-slate-600" />
              </div>
            </div>

            {/* Satellite speaker toggles */}
            {SATELLITES.map((s) => {
              const pos = ringPosition(s.azimuth);
              const on = speakers[s.key];
              return (
                <button
                  key={s.key}
                  type="button"
                  onClick={() => toggleSpeaker(s.key)}
                  aria-pressed={on}
                  aria-label={`${s.label} speaker`}
                  className="absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center gap-1.5"
                  style={pos}
                >
                  <span
                    className={cn(
                      "grid size-12 place-items-center rounded-full border transition-all",
                      on
                        ? "border-accent/50 bg-accent-muted text-accent-strong shadow-[0_0_18px] shadow-accent/30"
                        : "border-border bg-surface text-text-faint hover:text-text-muted",
                    )}
                  >
                    <Speaker className="size-5" aria-hidden="true" />
                  </span>
                  <span
                    className={cn(
                      "whitespace-nowrap rounded-control px-2 py-0.5 text-[11px] font-medium transition-colors",
                      on ? "text-text" : "text-text-faint",
                    )}
                  >
                    {s.label}
                  </span>
                </button>
              );
            })}

            {/* Subwoofer node */}
            <div
              className="absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center gap-1.5"
              style={ringPosition(180)}
            >
              <span
                className={cn(
                  "grid size-12 place-items-center rounded-full border transition-all",
                  subActive
                    ? "border-accent/50 bg-accent-muted text-accent-strong shadow-[0_0_18px] shadow-accent/30"
                    : "border-border bg-surface text-text-faint",
                )}
              >
                <Waves className="size-5" aria-hidden="true" />
              </span>
              <span
                className={cn(
                  "text-[11px] font-medium",
                  subActive ? "text-text" : "text-text-faint",
                )}
              >
                Subwoofer
              </span>
            </div>
          </div>
        </div>

        {/* Subwoofer level + actions */}
        <div className="flex items-center justify-between gap-6 border-t border-border px-6 py-4">
          <div className="flex flex-1 items-center gap-3">
            <span className="text-xs font-medium uppercase tracking-wider text-text-muted">
              Subwoofer
            </span>
            <Slider
              label="Subwoofer level"
              min={0}
              max={1}
              step={0.01}
              value={subwoofer}
              onChange={(v) => setSurround3d({ ...surround, enabled: true, subwoofer: v })}
              formatValue={(v) => `${Math.round(v * 100)} percent`}
              className="max-w-xs flex-1"
            />
            <span className="w-12 text-right text-xs tabular-nums text-text-muted">
              {Math.round(subwoofer * 100)}%
            </span>
          </div>
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}
