import { useEffect, useRef } from "react";
import { createFpsGate, useAudioData, useDprCanvas } from "./sceneKit";

const POINTS = 72;

/** Two-stage glow downscale: blob → 1/16, upscaled 1/16 → 1/4 → full. Each
 *  bilinear upscale smears over ~4 destination px, giving a soft ~20px halo
 *  for free (the old `shadowBlur ≈ 0.05 * minD` was a software-raster cliff). */
const GLOW_A_SCALE = 1 / 16;
const GLOW_B_SCALE = 1 / 4;

const blobGradient = (
  ctx: CanvasRenderingContext2D,
  cx: number,
  cy: number,
  baseR: number,
): CanvasGradient => {
  const grad = ctx.createRadialGradient(cx, cy, baseR * 0.2, cx, cy, baseR * 1.7);
  grad.addColorStop(0, "rgba(246,197,68,0.95)");
  grad.addColorStop(0.6, "rgba(120,200,90,0.6)");
  grad.addColorStop(1, "rgba(74,222,128,0.15)");
  return grad;
};

/**
 * Liquid Blob (2D) — a gooey closed shape whose radius is modulated by the
 * spectrum and slow wobble, pulsing on the beat. Smooth gold→green gradient fill
 * with a soft glow (rendered as a low-res upscale; the beat pulses its alpha,
 * visually equivalent to the old beat-scaled blur radius).
 */
export function LiquidBlob() {
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
    const radii: number[] = new Array(POINTS).fill(0);
    // Reused point buffer: every element is rewritten before it's read each
    // frame, so hoisting it out of the draw loop avoids POINTS allocations/frame.
    const pts: { x: number; y: number }[] = Array.from({ length: POINTS }, () => ({
      x: 0,
      y: 0,
    }));
    const glowA = document.createElement("canvas");
    const glowB = document.createElement("canvas");
    const gaCtx = glowA.getContext("2d")!;
    const gbCtx = glowB.getContext("2d")!;

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
      const cx = w / 2;
      const cy = h / 2;
      const minD = Math.min(w, h);
      const baseR = minD * 0.2 * (1 + a.beat * 0.12);
      const spec = a.spectrum;

      ctx.clearRect(0, 0, w, h);

      const half = POINTS / 2;
      for (let i = 0; i < POINTS; i++) {
        const ang = (i / POINTS) * Math.PI * 2;
        // Mirror the spectrum across the blob so it's symmetric.
        const band = i < half ? i : POINTS - i;
        const idx = spec.length > 0 ? Math.floor((band / half) * spec.length) : 0;
        const sv = spec.length > 0 ? spec[idx] ?? 0 : 0;
        const wob =
          Math.sin(ang * 3 + time * 1.3) * 0.06 + Math.sin(ang * 5 - time * 0.9) * 0.04;
        const target = baseR * (1 + sv * 0.55 + wob + a.bass * 0.3);
        radii[i] = (radii[i] ?? 0) + (target - (radii[i] ?? 0)) * 0.35;
        const r = radii[i] ?? baseR;
        const p = pts[i]!;
        p.x = cx + Math.cos(ang) * r;
        p.y = cy + Math.sin(ang) * r;
      }

      // Smooth closed path through the points (quadratic via midpoints), built
      // once and filled in both the glow and the crisp pass.
      const path = new Path2D();
      const first = pts[0]!;
      const lastP = pts[POINTS - 1]!;
      path.moveTo((lastP.x + first.x) / 2, (lastP.y + first.y) / 2);
      for (let i = 0; i < POINTS; i++) {
        const cur = pts[i]!;
        const nxt = pts[(i + 1) % POINTS]!;
        path.quadraticCurveTo(cur.x, cur.y, (cur.x + nxt.x) / 2, (cur.y + nxt.y) / 2);
      }
      path.closePath();

      // Glow pass: fill the blob tiny, upscale twice for a soft halo, with the
      // beat driving alpha (equivalent pulse to the old beat-scaled blur).
      const aw = Math.max(1, Math.ceil(w * GLOW_A_SCALE));
      const ah = Math.max(1, Math.ceil(h * GLOW_A_SCALE));
      const bw = Math.max(1, Math.ceil(w * GLOW_B_SCALE));
      const bh = Math.max(1, Math.ceil(h * GLOW_B_SCALE));
      if (glowA.width !== aw || glowA.height !== ah) {
        glowA.width = aw;
        glowA.height = ah;
      }
      if (glowB.width !== bw || glowB.height !== bh) {
        glowB.width = bw;
        glowB.height = bh;
      }
      gaCtx.setTransform(1, 0, 0, 1, 0, 0);
      gaCtx.clearRect(0, 0, aw, ah);
      gaCtx.setTransform(GLOW_A_SCALE, 0, 0, GLOW_A_SCALE, 0, 0);
      gaCtx.fillStyle = blobGradient(gaCtx, cx, cy, baseR);
      gaCtx.fill(path);
      gbCtx.clearRect(0, 0, bw, bh);
      gbCtx.drawImage(glowA, 0, 0, bw, bh);
      ctx.globalAlpha = Math.min(1, 0.5 * (0.6 + a.beat));
      ctx.drawImage(glowB, 0, 0, w, h);
      ctx.globalAlpha = 1;

      // Crisp blob on top.
      ctx.fillStyle = blobGradient(ctx, cx, cy, baseR);
      ctx.fill(path);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
