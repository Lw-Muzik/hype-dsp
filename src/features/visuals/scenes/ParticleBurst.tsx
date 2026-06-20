import { useEffect, useRef } from "react";
import { useAudioData, useDprCanvas } from "./sceneKit";

interface P {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  max: number;
}

const MAX_PARTICLES = 1400;

/**
 * Particle Burst (2D) — a particle field that explodes outward from the centre
 * on every beat (count + speed scale with loudness), then drifts and fades.
 * Uses a translucent-fill trail for motion blur.
 */
export function ParticleBurst() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { sample } = useAudioData();
  useDprCanvas(canvasRef);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let raf = 0;
    let last = performance.now();
    const particles: P[] = [];
    let prevBeat = 0;

    const draw = () => {
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const a = sample(dt);

      const w = canvas.width;
      const h = canvas.height;
      const cx = w / 2;
      const cy = h / 2;
      const minD = Math.min(w, h);

      // Fade the previous frame for a trail instead of clearing.
      ctx.fillStyle = "rgba(0,0,0,0.24)";
      ctx.fillRect(0, 0, w, h);

      if (a.beat > 0.5 && prevBeat <= 0.5) {
        const count = 40 + Math.floor(a.level * 90);
        for (let i = 0; i < count; i++) {
          const ang = Math.random() * Math.PI * 2;
          const spd = (0.25 + Math.random() * 0.75) * (0.18 + a.beat) * minD;
          particles.push({
            x: cx,
            y: cy,
            vx: Math.cos(ang) * spd,
            vy: Math.sin(ang) * spd,
            life: 0,
            max: 0.8 + Math.random() * 0.9,
          });
        }
      }
      prevBeat = a.beat;

      ctx.shadowBlur = 8;
      for (let k = particles.length - 1; k >= 0; k--) {
        const p = particles[k];
        if (!p) continue;
        p.life += dt;
        if (p.life >= p.max) {
          particles.splice(k, 1);
          continue;
        }
        p.x += p.vx * dt;
        p.y += p.vy * dt;
        p.vx *= 0.95;
        p.vy *= 0.95;
        const t = 1 - p.life / p.max;
        const r = ((146 * (1 - t)) | 0) + 100;
        const g = ((197 + 25 * (1 - t)) | 0);
        const b = ((68 + 60 * (1 - t)) | 0);
        const col = `rgba(${Math.min(246, r)},${g},${b},${t})`;
        ctx.fillStyle = col;
        ctx.shadowColor = col;
        ctx.beginPath();
        ctx.arc(p.x, p.y, 1.5 + t * 2.5, 0, Math.PI * 2);
        ctx.fill();
      }
      if (particles.length > MAX_PARTICLES) {
        particles.splice(0, particles.length - MAX_PARTICLES);
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
