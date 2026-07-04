import { useEffect, useRef } from "react";
import { createFpsGate, useAudioData, useDprCanvas } from "./sceneKit";

const BARS = 64;
/** Sprite padding reserved for the baked glow halo (sprite px). */
const GLOW_PAD = 24;
/** Baked glow blur — matches the old `shadowBlur = 12 * v` at full bar height. */
const GLOW_BLUR = 12;

interface Sprites {
  w: number;
  h: number;
  /** Bar body: the gold→green gradient, baked at full bar size. */
  bar: HTMLCanvasElement;
  /** Reflection: green→transparent fade (drawn with globalAlpha = v). */
  refl: HTMLCanvasElement;
  /** Glow: shadow-only gold silhouette of a bar, halo baked into the padding. */
  glow: HTMLCanvasElement;
  barW: number;
  barH: number;
}

/**
 * Bake the per-bar sprites once per canvas size. Replaces the old per-bar,
 * per-frame `createLinearGradient` + `shadowBlur` (a software-raster cliff on
 * WebKitGTK) with three tiny offscreen canvases drawn via `drawImage`. Scaling
 * the glow sprite with the bar height reproduces the old `shadowBlur = 12 * v`
 * spread; alpha modulation reproduces the beat bloom.
 */
function bakeSprites(w: number, h: number): Sprites {
  const barW = Math.max(1, Math.round(w / BARS - 2));
  const barH = Math.max(2, Math.round(h * 0.46));

  const bar = document.createElement("canvas");
  bar.width = barW;
  bar.height = barH;
  const bctx = bar.getContext("2d")!;
  const grad = bctx.createLinearGradient(0, 0, 0, barH);
  grad.addColorStop(0, "rgba(246,197,68,0.95)");
  grad.addColorStop(1, "rgba(74,222,128,0.55)");
  bctx.fillStyle = grad;
  bctx.fillRect(0, 0, barW, barH);

  const reflH = Math.max(1, Math.round(barH * 0.6));
  const refl = document.createElement("canvas");
  refl.width = barW;
  refl.height = reflH;
  const rctx = refl.getContext("2d")!;
  const rGrad = rctx.createLinearGradient(0, 0, 0, reflH);
  rGrad.addColorStop(0, "rgba(74,222,128,0.22)");
  rGrad.addColorStop(1, "rgba(74,222,128,0)");
  rctx.fillStyle = rGrad;
  rctx.fillRect(0, 0, barW, reflH);

  const glow = document.createElement("canvas");
  glow.width = barW + GLOW_PAD * 2;
  glow.height = barH + GLOW_PAD * 2;
  const gctx = glow.getContext("2d")!;
  // Shadow-only bake: draw the rect off-canvas and offset its shadow back in,
  // leaving just the blurred gold silhouette (the one shadowBlur pass we keep).
  const off = glow.width + 64;
  gctx.shadowColor = "rgba(246,197,68,0.6)";
  gctx.shadowBlur = GLOW_BLUR;
  gctx.shadowOffsetX = off;
  gctx.fillStyle = "#000";
  gctx.fillRect(GLOW_PAD - off, GLOW_PAD, barW, barH);

  return { w, h, bar, refl, glow, barW, barH };
}

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
    const gate = createFpsGate();
    const smooth: number[] = new Array(BARS).fill(0);
    let sprites: Sprites | null = null;

    const draw = () => {
      raf = requestAnimationFrame(draw);
      const now = performance.now();
      if (!gate(now)) return;
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const a = sample(dt);

      const w = canvas.width;
      const h = canvas.height;
      const mid = h * 0.64;
      const barW = w / BARS;
      const maxH = h * 0.46;
      const spec = a.spectrum;

      if (!sprites || sprites.w !== w || sprites.h !== h) {
        sprites = bakeSprites(w, h);
      }
      const { bar, refl, glow, barW: SW, barH: SH } = sprites;

      ctx.clearRect(0, 0, w, h);
      const glowStrength = a.beat * 0.5;
      const inflate = 1 + a.beat * 0.5;
      for (let i = 0; i < BARS; i++) {
        const idx = spec.length > 0 ? Math.floor((i / BARS) * spec.length) : -1;
        const target = idx >= 0 ? spec[idx] ?? 0 : 0;
        smooth[i] = (smooth[i] ?? 0) + (target - (smooth[i] ?? 0)) * 0.4;
        const v = Math.min(1, (smooth[i] ?? 0) + a.beat * 0.05);
        const bh = v * maxH;
        const x = i * barW + 1;
        const bw = Math.max(1, barW - 2);

        // Glow halo (baked sprite scaled to the bar; alpha ∝ v like the old
        // v-scaled blur, inflating + brightening on the beat).
        const sx = bw / SW;
        const sy = bh / SH;
        const dw = (SW + GLOW_PAD * 2) * sx * inflate;
        const dh = (SH + GLOW_PAD * 2) * sy * inflate;
        ctx.globalAlpha = Math.min(1, v + glowStrength);
        ctx.drawImage(glow, x + bw / 2 - dw / 2, mid - bh / 2 - dh / 2, dw, dh);

        // Bar body (gradient spans exactly the bar height, as before).
        ctx.globalAlpha = 1;
        ctx.drawImage(bar, x, mid - bh, bw, bh);

        // Reflection (no glow, fading down; alpha scaled by v as before).
        ctx.globalAlpha = v;
        ctx.drawImage(refl, x, mid, bw, bh * 0.6);
        ctx.globalAlpha = 1;
      }
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
