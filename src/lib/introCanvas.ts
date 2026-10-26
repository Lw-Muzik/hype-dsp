/**
 * Hype Muzik intro: procedural ripples -> blob -> mark pop -> lockup.
 * Framework-free; identical on mobile (Dart), desktop and web remote.
 * Timeline (s): ripples 0-1.6, collapse 1.6-2.2, pop 2.2-2.6,
 * slide + wordmark 2.6-3.2, hold to 3.6.
 */
export const INTRO_TOTAL = 3.6;
export const INTRO_FADE_MS = 250;

export interface IntroTheme {
  gold: string; // "r,g,b"
  ground: string;
  text: string;
  logo: CanvasImageSource | null;
  logoAspect: number; // width / height
  family: string;
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));
const easeOut = (x: number) => 1 - (1 - clamp(x)) ** 2;
const easeIn = (x: number) => clamp(x) ** 2;
const out3 = (x: number) => 1 - (1 - clamp(x)) ** 3;
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;

/** "#rrggbb" / "rgb(...)" -> "r,g,b" */
export function toRgb(css: string, fallback: string): string {
  const c = css.trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(c);
  if (hex) {
    const n = parseInt(hex[1]!, 16);
    return `${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}`;
  }
  const m = /rgba?\(([^)]+)\)/.exec(c);
  if (m) return m[1]!.split(/[ ,/]+/).slice(0, 3).join(",");
  return fallback;
}

function blob(
  ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number,
  phase: number, lobes: number, amp: number,
) {
  ctx.beginPath();
  const n = 72;
  for (let i = 0; i <= n; i++) {
    const a = (i / n) * Math.PI * 2;
    const rr = r * (1 + amp * Math.sin(lobes * a + phase) +
      amp * 0.5 * Math.sin((lobes + 1) * a - phase * 0.7));
    const x = cx + rr * Math.cos(a), y = cy + rr * Math.sin(a);
    if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
  }
  ctx.closePath();
}

/** Draw one frame at time t (seconds). `reduced` shows only the lockup. */
export function drawIntro(
  ctx: CanvasRenderingContext2D, w: number, h: number, t: number,
  th: IntroTheme, reduced = false,
) {
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = th.ground;
  ctx.fillRect(0, 0, w, h);
  const cx = w / 2, cy = h / 2, minD = Math.min(w, h);
  const gold = (a: number) => `rgba(${th.gold},${a})`;
  if (reduced) t = 99;

  if (!reduced) {
    const maxR = Math.hypot(w, h) * 0.4;
    const blobR = minD * 0.16;
    const k = easeIn((t - 1.6) / 0.6);
    for (let i = 0; i < 5; i++) {
      const s = i * 0.12;
      if (t <= s) continue;
      const p = clamp((t - s) / 1.2);
      const r = lerp(easeOut(p) * maxR * (1 - i * 0.08), blobR * (0.55 + 0.1 * i), k);
      const a = Math.min(1, p * 6) * (1 - 0.45 * p) * (1 - k);
      if (a <= 0.01 || r < 1) continue;
      blob(ctx, cx, cy, r, t * 0.9 + i * 1.3, i % 2 === 0 ? 3 : 4, 0.035 + (0.02 * i) / 4);
      const glow = ctx.createLinearGradient(0, cy - r, 0, cy + r);
      glow.addColorStop(0, gold(0));
      glow.addColorStop(1, gold(0.45 * a));
      ctx.lineWidth = 8;
      ctx.strokeStyle = glow;
      ctx.shadowColor = gold(0.5 * a);
      ctx.shadowBlur = 14;
      ctx.stroke();
      ctx.shadowBlur = 0;
      const g = ctx.createLinearGradient(0, cy - r, 0, cy + r);
      g.addColorStop(0, gold(0.12 * a));
      g.addColorStop(1, gold(a));
      ctx.lineWidth = 1.6;
      ctx.strokeStyle = g;
      ctx.stroke();
    }
    const blobA = k * (1 - clamp((t - 2.2) / 0.5));
    if (blobA > 0.01) {
      const r = blobR * (0.55 + 0.45 * k);
      blob(ctx, cx, cy, r, t * 1.4, 3, 0.06);
      const rg = ctx.createRadialGradient(cx, cy, 0, cx, cy, r * 1.6);
      rg.addColorStop(0, gold(0.55 * blobA));
      rg.addColorStop(1, gold(0));
      ctx.fillStyle = rg;
      ctx.shadowColor = gold(0.6 * blobA);
      ctx.shadowBlur = 24;
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = gold(0.8 * blobA);
      ctx.stroke();
    }
  }

  // Lockup
  const m = minD * 0.2;
  const mw = m * th.logoAspect;
  const fs = m * 0.62;
  ctx.font = `600 ${fs}px ${th.family}`;
  ctx.textBaseline = "middle";
  const tw = ctx.measureText("Hype Muzik").width;
  const gap = m * 0.22;
  const W = mw + gap + tw;
  const left = cx - W / 2;

  const pt = clamp((t - 2.2) / 0.4);
  const scale = pt < 0.6
    ? 0.6 + 0.48 * easeOut(pt / 0.6)
    : 1.08 - 0.08 * (0.5 - 0.5 * Math.cos(((pt - 0.6) / 0.4) * Math.PI));
  const u = out3((t - 2.6) / 0.6);
  const markCx = lerp(cx, left + mw / 2, u);

  ctx.globalAlpha = u;
  ctx.fillStyle = th.text;
  ctx.fillText("Hype Muzik", left + mw + gap - m * 0.3 * (1 - u), cy + fs * 0.04);
  ctx.globalAlpha = clamp((t - 2.2) / 0.12);
  if (th.logo && ctx.globalAlpha > 0) {
    ctx.save();
    ctx.translate(markCx, cy);
    ctx.scale(scale, scale);
    ctx.drawImage(th.logo, -mw / 2, -m / 2, mw, m);
    ctx.restore();
  }
  ctx.globalAlpha = 1;
}
