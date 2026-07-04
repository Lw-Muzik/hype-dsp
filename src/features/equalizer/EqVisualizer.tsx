import { useEffect, useMemo, useRef } from "react";
import { useEngineStore } from "@/stores/engine";
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

const SVG_NS = "http://www.w3.org/2000/svg";

/** Live EQ response curve overlaid on the real-time spectrum. */
export function EqVisualizer({ bands }: EqVisualizerProps) {
  const curve = useMemo(() => {
    const pts = bands.map((db, i) => ({
      x: xForFreq(ISO_CENTERS_HZ[i] ?? 20),
      y: yForDb(db),
    }));
    return smoothPath(pts);
  }, [bands]);

  // Real-time spectrum bars, driven by a transient store subscription and
  // imperative attribute writes on a pooled set of <rect>s — the ~30fps frames
  // never re-render the React tree (ScrollingWaveform-style). The rest of the
  // component re-renders only when the user edits the bands.
  const barsRef = useRef<SVGGElement>(null);
  useEffect(() => {
    const g = barsRef.current;
    if (!g) return;
    let count = -1;
    const render = (spectrum: number[]) => {
      const n = spectrum.length;
      if (n !== count) {
        count = n;
        g.textContent = "";
        const barWidth = n > 0 ? W / n : 0;
        for (let i = 0; i < n; i++) {
          const r = document.createElementNS(SVG_NS, "rect");
          r.setAttribute("x", String(i * barWidth));
          r.setAttribute("width", String(Math.max(0, barWidth - 1)));
          r.setAttribute("fill", "var(--color-accent)");
          r.setAttribute("opacity", "0.22");
          g.appendChild(r);
        }
      }
      const rects = g.children;
      for (let i = 0; i < n; i++) {
        const barHeight = (spectrum[i] ?? 0) * (H - 24);
        const rect = rects[i] as SVGRectElement;
        rect.setAttribute("y", String(H - barHeight));
        rect.setAttribute("height", String(barHeight));
      }
    };
    render(useEngineStore.getState().spectrum);
    return useEngineStore.subscribe((state, prev) => {
      if (state.spectrum !== prev.spectrum) render(state.spectrum);
    });
  }, []);

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

        {/* real-time spectrum bars (imperatively managed — see effect above) */}
        <g ref={barsRef} />

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
