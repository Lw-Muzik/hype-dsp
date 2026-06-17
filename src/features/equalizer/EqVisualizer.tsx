import { useMemo } from "react";
import { ISO_CENTERS_HZ } from "@/lib/types";
import { formatHz } from "@/lib/format";

const W = 1000;
const H = 220;
const PAD_Y = 14;
const DB_RANGE = 12;
const LOG_LO = Math.log10(20);
const LOG_HI = Math.log10(20_000);

interface EqVisualizerProps {
  bands: number[];
  spectrum: number[];
}

const xForFreq = (hz: number) =>
  ((Math.log10(hz) - LOG_LO) / (LOG_HI - LOG_LO)) * W;

const yForDb = (db: number) => {
  const amp = H / 2 - PAD_Y;
  return H / 2 - (Math.max(-DB_RANGE, Math.min(DB_RANGE, db)) / DB_RANGE) * amp;
};

interface Point {
  x: number;
  y: number;
}

/** Catmull-Rom → cubic-Bézier smoothing for a clean response curve. */
function smoothPath(points: Point[]): string {
  if (points.length < 2) return "";
  let d = `M ${points[0]!.x} ${points[0]!.y}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i - 1] ?? points[i]!;
    const p1 = points[i]!;
    const p2 = points[i + 1]!;
    const p3 = points[i + 2] ?? p2;
    const cp1x = p1.x + (p2.x - p0.x) / 6;
    const cp1y = p1.y + (p2.y - p0.y) / 6;
    const cp2x = p2.x - (p3.x - p1.x) / 6;
    const cp2y = p2.y - (p3.y - p1.y) / 6;
    d += ` C ${cp1x.toFixed(1)} ${cp1y.toFixed(1)}, ${cp2x.toFixed(1)} ${cp2y.toFixed(1)}, ${p2.x.toFixed(1)} ${p2.y.toFixed(1)}`;
  }
  return d;
}

const GRID_FREQS = [100, 1000, 10000];

/** Live EQ response curve overlaid on the real-time spectrum. */
export function EqVisualizer({ bands, spectrum }: EqVisualizerProps) {
  const curve = useMemo(() => {
    const pts = bands.map((db, i) => ({
      x: xForFreq(ISO_CENTERS_HZ[i] ?? 20),
      y: yForDb(db),
    }));
    return smoothPath(pts);
  }, [bands]);

  const barWidth = spectrum.length > 0 ? W / spectrum.length : 0;

  return (
    <div className="overflow-hidden rounded-control border border-border bg-surface">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        className="h-44 w-full"
        role="img"
        aria-label="Equalizer response curve over the audio spectrum"
      >
        {/* dB grid */}
        {[-6, 6].map((db) => (
          <line
            key={db}
            x1={0}
            x2={W}
            y1={yForDb(db)}
            y2={yForDb(db)}
            stroke="var(--color-border)"
            strokeWidth={1}
            strokeDasharray="2 6"
          />
        ))}
        <line
          x1={0}
          x2={W}
          y1={H / 2}
          y2={H / 2}
          stroke="var(--color-border-strong)"
          strokeWidth={1}
        />
        {/* frequency grid */}
        {GRID_FREQS.map((hz) => (
          <g key={hz}>
            <line
              x1={xForFreq(hz)}
              x2={xForFreq(hz)}
              y1={0}
              y2={H}
              stroke="var(--color-border)"
              strokeWidth={1}
              strokeDasharray="2 6"
            />
            <text
              x={xForFreq(hz) + 4}
              y={H - 6}
              fill="var(--color-text-faint)"
              fontSize={11}
            >
              {formatHz(hz)}
            </text>
          </g>
        ))}

        {/* real-time spectrum bars */}
        {spectrum.map((v, i) => {
          const barHeight = v * (H - 24);
          return (
            <rect
              key={i}
              x={i * barWidth}
              y={H - barHeight}
              width={Math.max(0, barWidth - 1)}
              height={barHeight}
              fill="var(--color-accent)"
              opacity={0.22}
            />
          );
        })}

        {/* response curve */}
        <path
          d={curve}
          fill="none"
          stroke="var(--color-accent-strong)"
          strokeWidth={2.5}
          strokeLinecap="round"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </div>
  );
}
