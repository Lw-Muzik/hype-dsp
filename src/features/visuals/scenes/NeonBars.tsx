import { useEffect, useRef } from "react";
import { useAudioData, useDprCanvas } from "./sceneKit";

const BARS = 64;

/**
 * Neon Bars (2D) — a linear frequency spectrum, gold→green, with a glow and a
 * mirrored reflection below the baseline. Bars overshoot and bloom on the beat.
 */
export function NeonBars() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { sample } = useAudioData();
  useDprCanvas(canvasRef);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let raf = 0;
    let last = performance.now();
    const smooth: number[] = new Array(BARS).fill(0);

    const draw = () => {
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const a = sample(dt);

      const w = canvas.width;
      const h = canvas.height;
      const mid = h * 0.64;
      const barW = w / BARS;
      const maxH = h * 0.46;
      const spec = a.spectrum;

      ctx.clearRect(0, 0, w, h);
      for (let i = 0; i < BARS; i++) {
        const idx = spec.length > 0 ? Math.floor((i / BARS) * spec.length) : -1;
        const target = idx >= 0 ? spec[idx] ?? 0 : 0;
        smooth[i] = (smooth[i] ?? 0) + (target - (smooth[i] ?? 0)) * 0.4;
        const v = Math.min(1, (smooth[i] ?? 0) + a.beat * 0.05);
        const bh = v * maxH;
        const x = i * barW + 1;
        const bw = Math.max(1, barW - 2);

        const grad = ctx.createLinearGradient(0, mid - bh, 0, mid);
        grad.addColorStop(0, "rgba(246,197,68,0.95)");
        grad.addColorStop(1, "rgba(74,222,128,0.55)");
        ctx.fillStyle = grad;
        ctx.shadowColor = "rgba(246,197,68,0.6)";
        ctx.shadowBlur = 12 * v + a.beat * 12;
        ctx.fillRect(x, mid - bh, bw, bh);

        // Reflection (no glow, fading down).
        ctx.shadowBlur = 0;
        const refl = ctx.createLinearGradient(0, mid, 0, mid + bh * 0.6);
        refl.addColorStop(0, `rgba(74,222,128,${0.22 * v})`);
        refl.addColorStop(1, "rgba(74,222,128,0)");
        ctx.fillStyle = refl;
        ctx.fillRect(x, mid, bw, bh * 0.6);
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
