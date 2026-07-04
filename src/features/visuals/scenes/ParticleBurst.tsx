import { useEffect, useRef } from "react";
import { createFpsGate, useAudioData, useDprCanvas } from "./sceneKit";

interface P {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  max: number;
}

const MAX_PARTICLES = 1400;
/** Life-progress buckets for the pre-baked particle sprites. */
const SPRITE_STEPS = 64;
const SPRITE_BLUR = 8;

interface Sprite {
  img: HTMLCanvasElement;
  half: number;
}

/**
 * Particle colour/size/glow are pure functions of life progress `t`, so bake
 * one glowing dot per `t` bucket once (the old per-particle `shadowBlur = 8`
 * arc, up to 1400 per frame, was a software-raster cliff on WebKitGTK). Each
 * sprite is rendered exactly like the old path — same fill, same shadow — so
 * a `drawImage` per particle is visually identical.
 */
function bakeSprites(): Sprite[] {
  const sprites: Sprite[] = [];
  for (let j = 0; j < SPRITE_STEPS; j++) {
    const t = (j + 0.5) / SPRITE_STEPS;
    const radius = 1.5 + t * 2.5;
    const r = ((146 * (1 - t)) | 0) + 100;
    const g = (197 + 25 * (1 - t)) | 0;
    const b = (68 + 60 * (1 - t)) | 0;
    const col = `rgba(${Math.min(246, r)},${g},${b},${t})`;

    const half = Math.ceil(radius + SPRITE_BLUR + 2);
    const img = document.createElement("canvas");
    img.width = half * 2;
    img.height = half * 2;
    const ctx = img.getContext("2d")!;
    ctx.fillStyle = col;
    ctx.shadowColor = col;
    ctx.shadowBlur = SPRITE_BLUR;
    ctx.beginPath();
    ctx.arc(half, half, radius, 0, Math.PI * 2);
    ctx.fill();
    sprites.push({ img, half });
  }
  return sprites;
}

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
    const gate = createFpsGate();
    const particles: P[] = [];
    let prevBeat = 0;
    const sprites = bakeSprites();

    const draw = () => {
      raf = requestAnimationFrame(draw);
      const now = performance.now();
      if (!gate(now)) return;
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
        const s = sprites[Math.min(SPRITE_STEPS - 1, (t * SPRITE_STEPS) | 0)]!;
        ctx.drawImage(s.img, p.x - s.half, p.y - s.half);
      }
      if (particles.length > MAX_PARTICLES) {
        particles.splice(0, particles.length - MAX_PARTICLES);
      }
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
