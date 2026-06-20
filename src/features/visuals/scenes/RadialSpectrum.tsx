import { useEffect, useRef } from "react";
import { useEngineStore } from "@/stores/engine";
import { useAudioData, useDprCanvas } from "./sceneKit";

const BARS = 96;
const GOLD = [246, 197, 68] as const;
const GREEN = [74, 222, 128] as const;

type RGB = readonly [number, number, number];
const mix = (a: RGB, b: RGB, t: number): RGB => [
  a[0] + (b[0] - a[0]) * t,
  a[1] + (b[1] - a[1]) * t,
  a[2] + (b[2] - a[2]) * t,
];

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
    const smooth: number[] = new Array(BARS).fill(0);
    const rings: { r: number; a: number }[] = [];
    let prevBeat = 0;

    const draw = () => {
      const now = performance.now();
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
      ctx.save();
      ctx.shadowColor = `rgba(246,197,68,${0.4 + 0.4 * audio.beat})`;
      ctx.shadowBlur = minDim * 0.06 * (0.6 + audio.beat);
      ctx.beginPath();
      ctx.arc(cx, cy, coreR, 0, Math.PI * 2);
      if (coverRef.current) {
        ctx.clip();
        ctx.drawImage(coverRef.current, cx - coreR, cy - coreR, coreR * 2, coreR * 2);
      } else {
        const grd = ctx.createRadialGradient(cx, cy, coreR * 0.2, cx, cy, coreR);
        grd.addColorStop(0, "rgba(246,197,68,0.9)");
        grd.addColorStop(1, "rgba(74,222,128,0.15)");
        ctx.fillStyle = grd;
        ctx.fill();
      }
      ctx.restore();

      raf = requestAnimationFrame(draw);
    };

    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
