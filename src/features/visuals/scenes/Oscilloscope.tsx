import { useEffect, useRef } from "react";
import { useAudioData, useDprCanvas } from "./sceneKit";

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
    const smooth: number[] = [];

    const draw = () => {
      const now = performance.now();
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
      ctx.lineWidth = Math.max(1.5, h * 0.004) * (1 + a.beat * 0.8);
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.shadowColor = "rgba(246,197,68,0.7)";
      ctx.shadowBlur = 10 + a.beat * 18;

      for (const dir of [-1, 1] as const) {
        ctx.strokeStyle = dir < 0 ? "rgba(246,197,68,0.9)" : "rgba(74,222,128,0.85)";
        ctx.beginPath();
        for (let i = 0; i < n; i++) {
          const raw = spec.length > 0 ? spec[i] ?? 0 : 0;
          // Idle shimmer so the line isn't flat in silence.
          const v = raw + (spec.length > 0 ? 0 : 0.02 * Math.sin(i * 0.4 + time * 3));
          smooth[i] = (smooth[i] ?? 0) + (v - (smooth[i] ?? 0)) * 0.5;
          const x = (i / (n - 1)) * w;
          const y = cy + dir * (smooth[i] ?? 0) * amp;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
