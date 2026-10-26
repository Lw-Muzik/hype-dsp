import { useEffect, useRef, useState } from "react";
import logoUrl from "@/assets/logo.png";
import { useAccountStore } from "@/stores/account";
import {
  INTRO_FADE_MS,
  INTRO_TOTAL,
  drawIntro,
  toRgb,
  type IntroTheme,
} from "@/lib/introCanvas";

const REDUCED_S = 0.6;

/**
 * Animated launch intro. The app mounts and loads underneath; the overlay
 * leaves at max(3.6 s, session check done). Click anywhere skips.
 */
export function IntroSplash() {
  const ready = useAccountStore((s) => !s.loading);
  const readyRef = useRef(ready);
  readyRef.current = ready;
  const [gone, setGone] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const skip = useRef(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    const root = rootRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !root || !ctx) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const cs = getComputedStyle(document.documentElement);
    const css = (v: string, d: string) => cs.getPropertyValue(v).trim() || d;
    const img = new Image();
    img.src = logoUrl;
    const theme: IntroTheme = {
      gold: toRgb(css("--color-accent", "#f5b40f"), "245,180,15"),
      ground: css("--color-canvas", "#0a0b0e"),
      text: css("--color-text", "#eceef2"),
      logo: null,
      logoAspect: 304 / 325,
      family: '"Clash Display", "Satoshi", system-ui, sans-serif',
    };
    img.onload = () => (theme.logo = img);
    void document.fonts?.load('600 40px "Clash Display"');

    let raf = 0;
    let fadeStart = 0;
    const t0 = performance.now();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const frame = (now: number) => {
      const w = window.innerWidth, h = window.innerHeight;
      if (canvas.width !== Math.round(w * dpr)) {
        canvas.width = Math.round(w * dpr);
        canvas.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const s = (now - t0) / 1000;
      drawIntro(ctx, w, h, s, theme, reduced);
      if (reduced) root.style.opacity = String(Math.min(1, s / 0.4));
      const total = reduced ? REDUCED_S : INTRO_TOTAL;
      if (!fadeStart && ((s >= total && readyRef.current) || skip.current)) {
        fadeStart = now;
      }
      if (fadeStart) {
        const k = (now - fadeStart) / INTRO_FADE_MS;
        root.style.opacity = String(Math.max(0, 1 - k));
        if (k >= 1) return setGone(true);
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, []);

  if (gone) return null;
  return (
    <div
      ref={rootRef}
      data-testid="intro-splash"
      onClick={() => (skip.current = true)}
      className="fixed inset-0 z-[9999] bg-canvas"
    >
      <canvas ref={canvasRef} className="block h-full w-full" />
    </div>
  );
}
