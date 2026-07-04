import { useEffect, useRef } from "react";
import { createFpsGate, useAudioData, useDprCanvas } from "./sceneKit";

/** Glow renders at 1/4 resolution — upscaling gives a free gaussian-ish blur. */
const GLOW_SCALE = 0.25;

/**
 * Oscilloscope (2D) — a symmetric neon line traced from the spectrum (mirrored
 * top/bottom around the centre), over a faint grid. Thickens and glows on the
 * beat for a retro scope look.
 */
export function Oscilloscope() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { sample } = useAudioData();
  useDprCanvas(canvasRef);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let raf = 0;
    let last = performance.now();
    let time = 0;
    const gate = createFpsGate();
    const smooth: number[] = [];
    // Hoisted per-frame point buffers (top line, bottom line, shared x).
    const xs: number[] = [];
    const ysTop: number[] = [];
    const ysBot: number[] = [];
    // Quarter-res offscreen canvas for the glow pass (the old two shadowBlur'd
    // polylines per frame were a software-raster cliff on WebKitGTK).
    const glow = document.createElement("canvas");
    const gctx = glow.getContext("2d")!;

    const tracePolyline = (
      target: CanvasRenderingContext2D,
      ys: number[],
      n: number,
    ) => {
      target.beginPath();
      for (let i = 0; i < n; i++) {
        if (i === 0) target.moveTo(xs[i]!, ys[i]!);
        else target.lineTo(xs[i]!, ys[i]!);
      }
      target.stroke();
    };

    const draw = () => {
      raf = requestAnimationFrame(draw);
      const now = performance.now();
      if (!gate(now)) return;
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      time += dt;
      const a = sample(dt);

      const w = canvas.width;
      const h = canvas.height;
      const cy = h / 2;
      ctx.clearRect(0, 0, w, h);

      // Faint grid.
      ctx.strokeStyle = "rgba(255,255,255,0.05)";
      ctx.lineWidth = 1;
      for (let gx = 0; gx <= 8; gx++) {
        const x = (gx / 8) * w;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, h);
        ctx.stroke();
      }
      ctx.beginPath();
      ctx.moveTo(0, cy);
      ctx.lineTo(w, cy);
      ctx.stroke();

      const spec = a.spectrum;
      const n = spec.length > 0 ? spec.length : 64;
      const amp = h * 0.42;
      const lw = Math.max(1.5, h * 0.004) * (1 + a.beat * 0.8);
      const blur = 10 + a.beat * 18;

      // Compute the two polylines (the bottom line intentionally smooths a
      // second time, exactly like the old per-dir loop did).
      xs.length = ysTop.length = ysBot.length = n;
      for (const dir of [-1, 1] as const) {
        const ys = dir < 0 ? ysTop : ysBot;
        for (let i = 0; i < n; i++) {
          const raw = spec.length > 0 ? spec[i] ?? 0 : 0;
          // Idle shimmer so the line isn't flat in silence.
          const v = raw + (spec.length > 0 ? 0 : 0.02 * Math.sin(i * 0.4 + time * 3));
          smooth[i] = (smooth[i] ?? 0) + (v - (smooth[i] ?? 0)) * 0.5;
          xs[i] = (i / (n - 1)) * w;
          ys[i] = cy + dir * (smooth[i] ?? 0) * amp;
        }
      }

      // Glow pass at quarter res, upscaled: a wide soft gold band whose width /
      // brightness track the old shadowBlur spread + energy dilution.
      const gw = Math.max(1, Math.ceil(w * GLOW_SCALE));
      const gh = Math.max(1, Math.ceil(h * GLOW_SCALE));
      if (glow.width !== gw || glow.height !== gh) {
        glow.width = gw;
        glow.height = gh;
      }
      gctx.setTransform(1, 0, 0, 1, 0, 0);
      gctx.clearRect(0, 0, gw, gh);
      gctx.setTransform(GLOW_SCALE, 0, 0, GLOW_SCALE, 0, 0);
      gctx.strokeStyle = "rgba(246,197,68,0.7)";
      gctx.globalAlpha = lw / (lw + blur);
      gctx.lineWidth = lw + blur * 1.2;
      gctx.lineCap = "round";
      gctx.lineJoin = "round";
      tracePolyline(gctx, ysTop, n);
      tracePolyline(gctx, ysBot, n);
      ctx.drawImage(glow, 0, 0, w, h);

      // Crisp lines on top.
      ctx.lineWidth = lw;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.strokeStyle = "rgba(246,197,68,0.9)";
      tracePolyline(ctx, ysTop, n);
      ctx.strokeStyle = "rgba(74,222,128,0.85)";
      tracePolyline(ctx, ysBot, n);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
