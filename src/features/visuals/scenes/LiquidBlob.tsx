import { useEffect, useRef } from "react";
import { useAudioData, useDprCanvas } from "./sceneKit";

const POINTS = 72;

/**
 * Liquid Blob (2D) — a gooey closed shape whose radius is modulated by the
 * spectrum and slow wobble, pulsing on the beat. Smooth gold→green gradient fill
 * with a soft glow.
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
    const radii: number[] = new Array(POINTS).fill(0);

    const draw = () => {
      const now = performance.now();
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

      const pts: { x: number; y: number }[] = [];
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
        pts.push({ x: cx + Math.cos(ang) * r, y: cy + Math.sin(ang) * r });
      }

      // Smooth closed path through the points (quadratic via midpoints).
      ctx.beginPath();
      const first = pts[0]!;
      const lastP = pts[POINTS - 1]!;
      ctx.moveTo((lastP.x + first.x) / 2, (lastP.y + first.y) / 2);
      for (let i = 0; i < POINTS; i++) {
        const cur = pts[i]!;
        const nxt = pts[(i + 1) % POINTS]!;
        ctx.quadraticCurveTo(cur.x, cur.y, (cur.x + nxt.x) / 2, (cur.y + nxt.y) / 2);
      }
      ctx.closePath();

      const grad = ctx.createRadialGradient(cx, cy, baseR * 0.2, cx, cy, baseR * 1.7);
      grad.addColorStop(0, "rgba(246,197,68,0.95)");
      grad.addColorStop(0.6, "rgba(120,200,90,0.6)");
      grad.addColorStop(1, "rgba(74,222,128,0.15)");
      ctx.fillStyle = grad;
      ctx.shadowColor = "rgba(246,197,68,0.5)";
      ctx.shadowBlur = minD * 0.05 * (0.6 + a.beat);
      ctx.fill();

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
