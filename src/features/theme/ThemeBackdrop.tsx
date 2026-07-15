import { useEffect, useRef, useState } from "react";
import { useEngineStore } from "@/stores/engine";
import { useThemeStore } from "@/stores/theme";
import { prefersReducedMotion } from "@/lib/reducedMotion";
import { backdropSource, type BackdropSource } from "./backdropSource";

const FADE_MS = 600;

/**
 * How long to hold the previous *real* cover art before giving up and
 * committing a seeded gradient. `itemMeta` (engine.ts) sets `cover: null` on
 * every track change, so `backdropSource` reports a gradient for a beat even
 * on tracks that DO have art — the real cover lands a moment later via
 * `applyNowPlaying` (local decode) or `fillNowPlayingCover` (cloud fetch).
 * 400ms is long enough to cover a local decode landing, or a fast cloud
 * fetch; it is NOT long enough to guarantee a slow cloud fetch lands in time
 * — that case still produces the old-art -> gradient -> new-art double fade
 * this hold exists to avoid. Accepted trade-off, not a bug: a track that
 * truly has no art must still resolve to its gradient reasonably promptly.
 */
const COVER_HOLD_MS = 400;

/**
 * Hard ceiling on simultaneously-mounted layers. Each commit also schedules
 * its own "prune anything below me" timer that fires once its own fade
 * completes, so in the common case the stack never holds more than two
 * layers at once. This cap only guards the pathological case of skip-spam
 * arriving faster than layers can settle.
 */
const MAX_LAYERS = 3;

interface StackLayer {
  /** Unique per commit (a monotonically increasing counter, never reused —
   *  see `layerIdRef` below). This is the React `key` and the identity
   *  `pruneBelow` matches against. It must never collide, which a
   *  source-derived string can: a quick skip forward and back (X->Y->X
   *  inside one fade window), or a `coverGradient` hash collision between
   *  two different art-less albums, can each produce two layers with the
   *  same source. The source string itself (`nextKey`/`committedKeyRef`)
   *  remains the *change-detection* key — it answers "did the underlying
   *  source actually change between renders?", a different question from
   *  "is this layer's identity unique?", which `id` answers. */
  id: number;
  source: BackdropSource;
  /** False only for the very first layer of a mount: it must paint solid
   *  from the first frame (see the component doc comment below) rather than
   *  fade in from nothing over the theme's plain surface. */
  animate: boolean;
}

/**
 * One art layer. Layers are permanently visible once mounted — the one-time
 * fade-in on mount (via CSS *animation*, not a transition; see the
 * component doc comment) is what produces the crossfade against whatever is
 * stacked beneath.
 */
function Layer({ source, animate }: { source: BackdropSource; animate: boolean }) {
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
        opacity: 1,
        // A CSS *animation* runs the instant a node is inserted into the DOM;
        // a *transition* needs a prior style value to animate away from and
        // does nothing on a freshly-mounted node. Each distinct source gets
        // its own layer with a stable key, mounted once and never mutated —
        // the fade-in animation itself IS the crossfade against the layer(s)
        // beneath, so nothing already on screen is ever touched mid-fade.
        animation: animate && !prefersReducedMotion ? `hm-backdrop-in ${FADE_MS}ms linear both` : undefined,
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
 *
 * Crossfades are driven by a *stack* of layers, newest on top, each one
 * fading in exactly once on mount via a CSS keyframe animation — not two
 * ping-ponging layers whose `background-image` gets reassigned under an
 * opacity *transition*. That older design had three bugs a stack fixes
 * structurally: transitions don't run on a freshly-mounted node (so the
 * first crossfade after mount was a hard cut); reassigning a hidden layer's
 * background while it might still be mid-fade-out pops visibly on rapid
 * skips; and it had no way to hold the previous art while the new track's
 * real cover was still decoding, so every track change flashed a seeded
 * gradient between two real covers. See the cover-pending hold below for
 * that last one.
 */
export default function ThemeBackdrop() {
  const resolved = useThemeStore((s) => s.resolved);
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const next = backdropSource(meta);
  const nextKey = keyOf(next);

  // The very first layer of a mount paints immediately (animate: false) —
  // but only when we already have real art to show: it has nothing beneath
  // it to crossfade from, so fading it in from nothing would flash the
  // theme's plain surface for 600ms. When the only thing available at mount
  // is a gradient, we deliberately do NOT seed it here — a gradient is
  // ambiguous (see COVER_HOLD_MS above), and painting it immediately would
  // flash a placeholder that a moment later gets replaced by real art: the
  // exact double-fade this file exists to prevent. Instead the stack starts
  // empty and the mount effect below applies the same cover-pending hold as
  // any other track change, so mount gets a brief empty backdrop (the
  // theme's plain surface) while it waits, rather than a placeholder
  // gradient. Every layer after the first fades in over whatever is already
  // stacked beneath it.
  const initialLayer: StackLayer | null =
    next && next.kind === "art" ? { id: 0, source: next, animate: false } : null;

  const [stack, setStack] = useState<StackLayer[]>(() => (initialLayer ? [initialLayer] : []));

  // Source string of what's actually committed (pushed onto the stack), as
  // opposed to `next`, which reflects the store on every render including
  // while a cover-pending hold is outstanding. Purely for change detection —
  // "did the source change since the last commit?" — never used as a React
  // key (see `StackLayer.id` for why that has to be a separate, always-
  // unique value). Left "" when nothing has been committed yet, which is
  // also true right after a mount whose only available source was a
  // gradient and got deferred to the hold below: "" mismatches `nextKey` on
  // the very next effect run, so that deferred mount is picked up by the
  // same hold logic as any other change, not a special case.
  const committedKeyRef = useRef(initialLayer ? nextKey : "");
  // Hands out each layer's unique `id`. 0 is reserved for the initial
  // (mount-time) layer above; every layer committed after that gets the
  // next integer, so ids never collide no matter how the source strings
  // collide (see `StackLayer.id`).
  const layerIdRef = useRef(0);
  const holdTimerRef = useRef<number | null>(null);
  // Every outstanding setTimeout (the cover-pending hold, plus one
  // prune-below-me per commit), so they can all be cancelled on unmount and
  // never call setState after.
  const timersRef = useRef<Set<number>>(new Set());

  const schedule = (fn: () => void, ms: number): number => {
    const id = window.setTimeout(() => {
      timersRef.current.delete(id);
      fn();
    }, ms);
    timersRef.current.add(id);
    return id;
  };
  const cancel = (id: number | null) => {
    if (id == null) return;
    window.clearTimeout(id);
    timersRef.current.delete(id);
  };

  /** Push `source` as the new top layer, capped, and arrange for whatever is
   *  now permanently hidden beneath it to be dropped from the DOM.
   *  `sourceKey` is the change-detection string (see `committedKeyRef`); the
   *  layer's actual identity is minted fresh here from `layerIdRef` and
   *  never derived from `sourceKey`, so it can't collide even when two
   *  layers share a source. */
  const commit = (source: BackdropSource, sourceKey: string) => {
    committedKeyRef.current = sourceKey;
    const id = ++layerIdRef.current;
    setStack((prev) => {
      const appended: StackLayer[] = [...prev, { id, source, animate: prev.length > 0 }];
      return appended.length > MAX_LAYERS ? appended.slice(appended.length - MAX_LAYERS) : appended;
    });
    const pruneBelow = () => {
      setStack((prev) => {
        const idx = prev.findIndex((l) => l.id === id);
        // idx < 0: already pruned by a later commit. idx === 0: nothing
        // beneath it to drop. Either way, leave the stack alone.
        return idx > 0 ? prev.slice(idx) : prev;
      });
    };
    // Reduced motion has no fade to wait for — the swap is instant, so
    // whatever was beneath is already fully hidden; prune synchronously.
    // Otherwise wait for this layer's own fade-in to finish covering it.
    if (prefersReducedMotion) pruneBelow();
    else schedule(pruneBelow, FADE_MS);
  };

  useEffect(() => {
    if (nextKey === committedKeyRef.current) return;
    cancel(holdTimerRef.current);
    holdTimerRef.current = null;

    if (!next) {
      // Nothing playing: paint nothing, per backdropSource's contract.
      committedKeyRef.current = "";
      setStack([]);
      return;
    }

    // Cover-pending hold: a gradient is ambiguous — it might mean the track
    // genuinely has no art, or it might just mean the real cover hasn't
    // decoded yet (`itemMeta` sets `cover: null` on every track change; the
    // real cover, if any, lands a beat later via `applyNowPlaying` or
    // `fillNowPlayingCover`). So an incoming gradient is never committed
    // immediately, regardless of what — if anything — is already on screen;
    // depending on what's currently shown would miss the mirror-image case
    // (an art-less track followed by one that DOES have art, where nothing
    // "currently showing art" would otherwise skip the hold and still
    // double-fade). If the real cover arrives before the hold expires,
    // `next` becomes an `art` source with a different key, this effect
    // re-runs, and we commit that directly (one crossfade: whatever's on
    // screen -> new art, or a plain solid paint if the stack is still
    // empty). If the hold expires first, the track genuinely has no art, so
    // the gradient commits late rather than never.
    if (next.kind === "gradient") {
      const pending = next;
      const pendingKey = nextKey;
      holdTimerRef.current = schedule(() => {
        holdTimerRef.current = null;
        commit(pending, pendingKey);
      }, COVER_HOLD_MS);
      return;
    }

    commit(next, nextKey);
    // `next` is a fresh object every render; `nextKey` already captures the
    // only change that matters, and `next` from this same render is what
    // the effect body closes over.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nextKey]);

  // Cancel every outstanding timer on unmount so none of them call setState
  // after.
  useEffect(() => {
    return () => {
      for (const id of timersRef.current) window.clearTimeout(id);
      timersRef.current.clear();
    };
  }, []);

  if (resolved !== "dynamic") return null;

  return (
    <div className="pointer-events-none absolute inset-0 -z-10 overflow-hidden" aria-hidden="true">
      {stack.map((l) => (
        <Layer key={l.id} source={l.source} animate={l.animate} />
      ))}
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
