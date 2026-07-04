import { useEffect, useRef } from "react";
import { useEngineStore } from "@/stores/engine";
import { createFpsGate, useAudioData, useDprCanvas } from "./sceneKit";

const BARS = 96;
const GOLD = [246, 197, 68] as const;
const GREEN = [74, 222, 128] as const;

type RGB = readonly [number, number, number];
// A single reused scratch so mixing a colour per bar per frame doesn't allocate
// a fresh array each time (the result is read out immediately at the call site).
const mixScratch: [number, number, number] = [0, 0, 0];
const mix = (a: RGB, b: RGB, t: number): RGB => {
  mixScratch[0] = a[0] + (b[0] - a[0]) * t;
  mixScratch[1] = a[1] + (b[1] - a[1]) * t;
  mixScratch[2] = a[2] + (b[2] - a[2]) * t;
  return mixScratch;
};

interface CoreSprites {
  w: number;
  h: number;
  /** Baked circle radius (device px) the sprites were rendered at. */
  R: number;
  /** The brand radial-gradient core, no glow. */
  core: HTMLCanvasElement;
  /** Shadow-only gold halo of the gradient core (alpha baked at 1). */
  halo: HTMLCanvasElement;
}

/**
 * Bake the gradient core + its gold halo once per canvas size. The old path
 * re-rendered a `shadowBlur ≈ 0.06 * minDim` (~190px on large windows) every
 * frame — a software-raster cliff on WebKitGTK. The halo is baked from the
 * same gradient fill the old shadow derived from; the per-frame beat pulse is
 * reproduced by scaling (radius) and globalAlpha (the old shadow alpha ramp).
 */
function bakeCore(w: number, h: number): CoreSprites {
  const minDim = Math.min(w, h);
  const R = Math.max(2, Math.ceil(minDim * 0.2));
  const blur = minDim * 0.06;

  const core = document.createElement("canvas");
  core.width = core.height = R * 2;
  const cctx = core.getContext("2d")!;
  const grd = cctx.createRadialGradient(R, R, R * 0.2, R, R, R);
  grd.addColorStop(0, "rgba(246,197,68,0.9)");
  grd.addColorStop(1, "rgba(74,222,128,0.15)");
  cctx.fillStyle = grd;
  cctx.beginPath();
  cctx.arc(R, R, R, 0, Math.PI * 2);
  cctx.fill();

  const pad = Math.ceil(blur * 2);
  const halo = document.createElement("canvas");
  halo.width = halo.height = (R + pad) * 2;
  const hctx = halo.getContext("2d")!;
  const c = R + pad;
  // Shadow-only bake: fill the gradient circle off-canvas and offset its
  // shadow back in, leaving just the blurred gold halo silhouette.
  const off = halo.width + 64;
  const hgrd = hctx.createRadialGradient(c - off, c, R * 0.2, c - off, c, R);
  hgrd.addColorStop(0, "rgba(246,197,68,0.9)");
  hgrd.addColorStop(1, "rgba(74,222,128,0.15)");
  hctx.shadowColor = "rgba(246,197,68,1)";
  hctx.shadowBlur = blur;
  hctx.shadowOffsetX = off;
  hctx.fillStyle = hgrd;
  hctx.beginPath();
  hctx.arc(c - off, c, R, 0, Math.PI * 2);
  hctx.fill();

  return { w, h, R, core, halo };
}

/**
 * Radial Spectrum (2D) — frequency bars fanned in a full circle around the
 * album art, gold→green by energy, with a beat-driven core pulse and expanding
 * shockwave rings. Pure Canvas 2D; runs only while mounted.
 */
export function RadialSpectrum() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { sample } = useAudioData();
  useDprCanvas(canvasRef);

  const coverRef = useRef<HTMLImageElement | null>(null);
  const coverSrcRef = useRef<string | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    let raf = 0;
    let last = performance.now();
    const gate = createFpsGate();
    const smooth: number[] = new Array(BARS).fill(0);
    const rings: { r: number; a: number }[] = [];
    let prevBeat = 0;
    let baked: CoreSprites | null = null;
    // Cover art pre-scaled to the core's max size once per track/resize, so
    // the per-frame draw isn't a full-res image resample.
    let coverScaled: HTMLCanvasElement | null = null;
    let coverScaledFrom: HTMLImageElement | null = null;
    let coverScaledSize = 0;

    const draw = () => {
      raf = requestAnimationFrame(draw);
      const now = performance.now();
      if (!gate(now)) return;
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const audio = sample(dt);

      // Lazy-load the track's cover art when it changes.
      const cover = useEngineStore.getState().nowPlayingMeta?.cover ?? null;
      if (cover !== coverSrcRef.current) {
        coverSrcRef.current = cover;
        if (cover) {
          const img = new Image();
          img.onload = () => (coverRef.current = img);
          img.src = cover;
        } else {
          coverRef.current = null;
        }
      }

      const w = canvas.width;
      const h = canvas.height;
      const cx = w / 2;
      const cy = h / 2;
      const minDim = Math.min(w, h);
      const inner = minDim * 0.2;
      const maxLen = minDim * 0.26;

      if (!baked || baked.w !== w || baked.h !== h) baked = bakeCore(w, h);

      // Keep the pre-scaled cover in sync with the image + core size.
      const img = coverRef.current;
      if (!img) {
        coverScaled = null;
        coverScaledFrom = null;
      } else {
        const size = Math.max(2, Math.ceil(inner * 1.04) * 2);
        if (coverScaledFrom !== img || coverScaledSize !== size) {
          const c = document.createElement("canvas");
          c.width = c.height = size;
          c.getContext("2d")!.drawImage(img, 0, 0, size, size);
          coverScaled = c;
          coverScaledFrom = img;
          coverScaledSize = size;
        }
      }

      ctx.clearRect(0, 0, w, h);

      const spec = audio.spectrum;
      ctx.lineCap = "round";
      ctx.lineWidth = Math.max(2, minDim * 0.006);
      for (let i = 0; i < BARS; i++) {
        const idx = spec.length > 0 ? Math.floor((i / BARS) * spec.length) : -1;
        const target = idx >= 0 ? spec[idx] ?? 0 : 0;
        smooth[i] = (smooth[i] ?? 0) + (target - (smooth[i] ?? 0)) * 0.35;
        const v = smooth[i] ?? 0;
        const ang = (i / BARS) * Math.PI * 2 - Math.PI / 2;
        const len = inner + v * maxLen + audio.beat * minDim * 0.02;
        const [r, g, b] = mix(GOLD, GREEN, Math.min(1, v));
        ctx.strokeStyle = `rgba(${r | 0},${g | 0},${b | 0},${0.35 + 0.65 * v})`;
        ctx.beginPath();
        ctx.moveTo(cx + Math.cos(ang) * inner, cy + Math.sin(ang) * inner);
        ctx.lineTo(cx + Math.cos(ang) * len, cy + Math.sin(ang) * len);
        ctx.stroke();
      }

      // Beat shockwave rings.
      if (audio.beat > 0.5 && prevBeat <= 0.5) rings.push({ r: inner, a: 0.5 });
      prevBeat = audio.beat;
      ctx.lineWidth = Math.max(1.5, minDim * 0.004);
      for (let k = rings.length - 1; k >= 0; k--) {
        const ring = rings[k];
        if (!ring) continue;
        ring.r += dt * minDim * 0.9;
        ring.a -= dt * 0.9;
        if (ring.a <= 0) {
          rings.splice(k, 1);
          continue;
        }
        ctx.strokeStyle = `rgba(246,197,68,${ring.a})`;
        ctx.beginPath();
        ctx.arc(cx, cy, ring.r, 0, Math.PI * 2);
        ctx.stroke();
      }

      // Centre core — cover art or brand gradient — pulsing on the beat.
      const coreR = inner * (0.92 + audio.beat * 0.12);
      if (coverScaled) {
        // The old shadow was clipped away here (invisible), so no halo drawn.
        ctx.save();
        ctx.beginPath();
        ctx.arc(cx, cy, coreR, 0, Math.PI * 2);
        ctx.clip();
        ctx.drawImage(coverScaled, cx - coreR, cy - coreR, coreR * 2, coreR * 2);
        ctx.restore();
      } else {
        const s = coreR / baked.R;
        const dw = baked.halo.width * s;
        ctx.globalAlpha = 0.4 + 0.4 * audio.beat;
        ctx.drawImage(baked.halo, cx - dw / 2, cy - dw / 2, dw, dw);
        ctx.globalAlpha = 1;
        ctx.drawImage(baked.core, cx - coreR, cy - coreR, coreR * 2, coreR * 2);
      }
    };

    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
