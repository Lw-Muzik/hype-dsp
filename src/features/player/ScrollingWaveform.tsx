import { useEffect, useRef } from "react";
import { useEngineStore } from "@/stores/engine";

/** Device-independent width of one sample column (CSS px). */
const COL_W = 2;

/** A fixed-length ring buffer of recent stereo levels (one slot per frame). */
interface Ring {
  l: Float32Array;
  r: Float32Array;
  /** Index the next sample will be written to (oldest sample lives here). */
  head: number;
  len: number;
}

const clamp01 = (v: number): number => (v > 1 ? 1 : v < 0 ? 0 : v);

/** Redraw the whole ring buffer: oldest at the left, newest at the right. */
function drawWaveform(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  buf: Ring | null,
): void {
  ctx.clearRect(0, 0, width, height);
  if (!buf) return;
  const dpr = window.devicePixelRatio || 1;
  const colW = COL_W * dpr;
  const mid = height / 2;

  for (let x = 0; x < buf.len; x++) {
    const idx = (buf.head + x) % buf.len;
    const lv = buf.l[idx]!;
    const rv = buf.r[idx]!;
    const px = x * colW;
    // Top half = left channel (blue); bottom half = right channel (red),
    // brighter as the level rises — VirtualDJ's stereo signature.
    if (lv > 0.001) {
      ctx.fillStyle = `rgba(56, 140, 255, ${0.3 + 0.7 * lv})`;
      ctx.fillRect(px, mid - lv * mid, colW + 0.6, lv * mid);
    }
    if (rv > 0.001) {
      ctx.fillStyle = `rgba(240, 60, 70, ${0.3 + 0.7 * rv})`;
      ctx.fillRect(px, mid, colW + 0.6, rv * mid);
    }
  }
  // Faint centre seam.
  ctx.fillStyle = "rgba(255, 255, 255, 0.05)";
  ctx.fillRect(0, mid - dpr * 0.5, width, dpr);
}

/** The canvas + its data pump. Mounted only while a track is loaded, so it
 *  starts fresh each session and tears down cleanly on stop. */
function WaveformCanvas() {
  // Re-render only on pause toggle (cheap) to start/stop the draw loop.
  const paused = useEngineStore((s) => s.paused);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const bufRef = useRef<Ring | null>(null);

  // Size the canvas (DPR-aware) and (re)allocate the ring buffer to its width.
  useEffect(() => {
    const canvas = canvasRef.current;
    const parent = canvas?.parentElement;
    if (!canvas || !parent) return;
    const ctx = canvas.getContext("2d");

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.floor(parent.clientWidth));
      const h = Math.max(1, Math.floor(parent.clientHeight));
      canvas.width = Math.floor(w * dpr);
      canvas.height = Math.floor(h * dpr);
      const cols = Math.max(8, Math.floor(w / COL_W));
      bufRef.current = {
        l: new Float32Array(cols),
        r: new Float32Array(cols),
        head: 0,
        len: cols,
      };
      // Setting width clears the canvas; repaint once so a resize while paused
      // doesn't leave it blank.
      if (ctx) drawWaveform(ctx, canvas.width, canvas.height, bufRef.current);
    };

    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(parent);
    return () => ro.disconnect();
  }, []);

  // Push one column per engine frame while running — transient store
  // subscription, so this never triggers a React render.
  useEffect(() => {
    return useEngineStore.subscribe((state, prev) => {
      if (state.meters === prev.meters) return; // only on a fresh frame
      if (!(state.playing && !state.paused)) return; // frozen while paused
      const buf = bufRef.current;
      if (!buf) return;
      const { peak, rms } = state.meters;
      // Blend RMS (body) with peak (transients) for a fuller envelope.
      buf.l[buf.head] = clamp01(Math.max((rms[0] ?? 0) * 1.25, peak[0] ?? 0));
      buf.r[buf.head] = clamp01(Math.max((rms[1] ?? 0) * 1.25, peak[1] ?? 0));
      buf.head = (buf.head + 1) % buf.len;
    });
  }, []);

  // Draw loop — runs while playing, stops when paused (leaving the waveform
  // frozen in place, like VirtualDJ).
  useEffect(() => {
    if (paused) return;
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let raf = 0;
    const draw = () => {
      drawWaveform(ctx, canvas.width, canvas.height, bufRef.current);
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [paused]);

  return (
    <div className="relative size-full overflow-hidden rounded-md bg-black/25 ring-1 ring-white/5">
      <canvas
        ref={canvasRef}
        className="block size-full [filter:blur(0.4px)]"
        aria-hidden="true"
      />
    </div>
  );
}

/**
 * A VirtualDJ-style scrolling stereo waveform that fills its container (mounted
 * in the top bar): left channel sweeps the top in blue, right channel the bottom
 * in red, newest at the right edge. It runs while audio plays and freezes when
 * paused. Renders nothing when no track is loaded.
 */
export function ScrollingWaveform() {
  const playing = useEngineStore((s) => s.playing);
  if (!playing) return null;
  return <WaveformCanvas />;
}
