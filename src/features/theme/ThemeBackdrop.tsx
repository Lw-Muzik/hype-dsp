import { useEffect, useRef, useState } from "react";
import { useEngineStore } from "@/stores/engine";
import { useThemeStore } from "@/stores/theme";
import { prefersReducedMotion } from "@/lib/reducedMotion";
import { backdropSource, type BackdropSource } from "./backdropSource";

const FADE_MS = 600;

/** One art layer. `show` drives the crossfade. */
function Layer({ source, show }: { source: BackdropSource | null; show: boolean }) {
  if (!source) return null;
  const art =
    source.kind === "art"
      ? { backgroundImage: `url("${source.url}")`, backgroundSize: "cover", backgroundPosition: "center" }
      : { background: source.css };
  return (
    // The wrapper is promoted, NOT the blurred child. Promoting the blurred
    // element would force the GPU to re-blur its texture every frame of the
    // fade; promoting the parent lets the blur rasterise once and be reused.
    <div
      className="absolute inset-0"
      style={{
        willChange: "transform",
        opacity: show ? 1 : 0,
        // Linear: an eased crossfade dips visibly in the middle.
        transition: prefersReducedMotion ? undefined : `opacity ${FADE_MS}ms linear`,
      }}
      aria-hidden="true"
    >
      <div
        className="absolute"
        style={{
          // blur()'s length is a standard deviation, so it bleeds ~3x that far
          // and samples transparent pixels past the edge, fading them. Oversize
          // by 3σ to put the fade off-screen. (scale() would magnify it, since
          // transform applies after filter.)
          inset: "calc(var(--hm-backdrop-blur) * -3)",
          // saturate AFTER blur: blur averages colours toward grey, and this
          // restores the chroma that averaging removed. Filters apply
          // left-to-right, which is why this is inline and not Tailwind classes.
          filter: "blur(var(--hm-backdrop-blur)) saturate(1.5)",
          ...art,
        }}
      />
    </div>
  );
}

/**
 * The Dynamic theme's cover-art backdrop.
 *
 * Mounted as a negative-z child of the isolated root, so it paints above the
 * root's own background and below every piece of chrome — which then reveals it
 * purely through translucent surface tokens. Renders nothing in other themes.
 */
export default function ThemeBackdrop() {
  const resolved = useThemeStore((s) => s.resolved);
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const next = backdropSource(meta);

  // A/B ping-pong. `cover` is null for a beat after every track change while
  // tags decode, so we hold the previous art rather than flashing empty.
  const [layers, setLayers] = useState<{ a: BackdropSource | null; b: BackdropSource | null; showA: boolean }>({
    a: next, b: null, showA: true,
  });
  const lastKey = useRef(keyOf(next));

  useEffect(() => {
    const key = keyOf(next);
    if (key === lastKey.current) return;
    lastKey.current = key;
    setLayers((prev) =>
      prev.showA ? { a: prev.a, b: next, showA: false } : { a: next, b: prev.b, showA: true },
    );
  }, [next]);

  if (resolved !== "dynamic") return null;

  return (
    <div className="pointer-events-none absolute inset-0 -z-10 overflow-hidden" aria-hidden="true">
      <Layer source={layers.a} show={layers.showA} />
      <Layer source={layers.b} show={!layers.showA} />
      {/* Single darkening step. Art opacity AND a scrim would multiply, crushing
          peak white to ~31/255 and guaranteeing banding. The value lives in
          index.css as --hm-backdrop-scrim, which palette.test.ts asserts on. */}
      <div className="absolute inset-0" style={{ background: "var(--hm-backdrop-scrim)" }} />
      {/* Dither. 71 levels of blurred gradient bands on 8-bit displays. Must be
          unscaled and unblurred, or it stops working as per-pixel noise. */}
      <div className="hm-grain absolute inset-0" />
    </div>
  );
}

function keyOf(s: BackdropSource | null): string {
  if (!s) return "";
  return s.kind === "art" ? s.url : s.css;
}
