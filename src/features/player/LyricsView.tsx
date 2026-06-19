import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Loader2, MicVocal } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { useLyrics } from "@/features/player/useLyrics";
import { activeLineIndex } from "@/lib/lrc";
import type { LyricLine } from "@/lib/lrc";
import { cn } from "@/lib/cn";

const reduceMotion =
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Where the active line sits, as a fraction of the viewport height. */
const FOCAL = 0.4;
/** Framer-style gentle spring (stiffness/damping/mass) for the auto-scroll. */
const STIFFNESS = 170;
const DAMPING = 26;

interface TimedWord {
  text: string;
  startMs: number;
  endMs: number;
}

/**
 * Split a synced line into timed words. Uses real Enhanced-LRC word timing when
 * the source provides it; otherwise it paces the line's own words across its
 * duration (weighted by length) so the fill still tracks the music and wraps
 * naturally — the closest we can get to Apple's karaoke without word data.
 */
function timedWords(line: LyricLine): TimedWord[] {
  const start = line.timeMs ?? 0;
  const end = line.endMs ?? start + 4000;
  if (line.words?.length) {
    return line.words.map((w, i) => ({
      text: w.text,
      startMs: w.timeMs,
      endMs: line.words![i + 1]?.timeMs ?? end,
    }));
  }
  const words = line.text.split(/\s+/).filter(Boolean);
  const totalChars = words.reduce((n, w) => n + w.length, 0) || 1;
  const span = Math.max(end - start, 400);
  let acc = start;
  return words.map((w) => {
    const from = acc;
    acc += span * (w.length / totalChars);
    return { text: w, startMs: from, endMs: acc };
  });
}

/**
 * Lyrics for the current track, in the Apple-Music style: a spring-driven
 * auto-scroll keeps the active line at the focal point, past/distant lines
 * recede with a depth-of-field blur, and the active line fills word-by-word in
 * time with the music. Synced LRC (incl. Enhanced word timing) drives all of
 * this; plain lyrics fall back to readable scrollable text. Tap a line to seek.
 */
export function LyricsView() {
  const { loading, lyrics } = useLyrics();
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const seekable = useEngineStore((s) => s.seekable);
  const seek = useEngineStore((s) => s.seek);

  const [active, setActive] = useState(-1);
  const [viewportH, setViewportH] = useState(480);

  const viewportRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLParagraphElement>(null);

  // Live values read inside the rAF loop without forcing React re-renders.
  const focal = viewportH * FOCAL;
  const focalRef = useRef(focal);
  focalRef.current = focal;
  const activeRefIdx = useRef(active);
  activeRefIdx.current = active;

  const synced = !!lyrics?.synced;
  const offsetMs = lyrics?.offsetMs ?? 0;
  const lines = lyrics?.lines;

  const activeWords = useMemo(
    () => (synced && active >= 0 && lines?.[active] ? timedWords(lines[active]!) : null),
    [synced, active, lines],
  );

  // Track the scroll viewport height so the focal point + spacers stay right.
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => {
      if (e) setViewportH(e.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // The animation loop: interpolate position, advance the active line, spring
  // the scroll, and fill the active line's words — all imperatively per frame.
  useEffect(() => {
    if (!synced || !lines) return;
    let raf = 0;
    let last = performance.now();
    // Spring state for the list's translateY.
    let y = 0;
    let vel = 0;
    // Local position interpolation between ~10fps progress events.
    let basePos = -1;
    let baseClock = last;

    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 0.064);
      last = now;

      const st = useEngineStore.getState();
      if (st.positionSecs !== basePos) {
        basePos = st.positionSecs;
        baseClock = now;
      }
      const advance = st.playing && !st.paused ? (now - baseClock) / 1000 : 0;
      let posSecs = basePos + advance;
      if (st.durationSecs != null) posSecs = Math.min(posSecs, st.durationSecs);
      const posMs = posSecs * 1000 - offsetMs;

      // Active line.
      const idx = activeLineIndex(lines, posMs);
      if (idx !== activeRefIdx.current) {
        activeRefIdx.current = idx;
        setActive(idx);
      }

      // Spring the scroll toward the active line's centre at the focal point.
      const el = activeRef.current;
      let target = y;
      if (el && idx >= 0) {
        target = focalRef.current - (el.offsetTop + el.offsetHeight / 2);
      } else if (idx < 0) {
        target = 0;
      }
      if (reduceMotion) {
        y = target;
        vel = 0;
      } else {
        const accel = STIFFNESS * (target - y) - DAMPING * vel;
        vel += accel * dt;
        y += vel * dt;
        if (Math.abs(target - y) < 0.4 && Math.abs(vel) < 0.4) {
          y = target;
          vel = 0;
        }
      }
      if (listRef.current) {
        listRef.current.style.transform = `translate3d(0, ${y.toFixed(2)}px, 0)`;
      }

      // Word-by-word fill of the active line (DOM matches the rendered index).
      const lineEl = activeRef.current;
      if (lineEl && lineEl.dataset.idx === String(idx)) {
        const spans = lineEl.querySelectorAll<HTMLElement>("[data-w]");
        for (const span of spans) {
          const ws = Number(span.dataset.start);
          const we = Number(span.dataset.end);
          if (posMs >= we) {
            if (span.dataset.s !== "past") span.dataset.s = "past";
          } else if (posMs >= ws) {
            span.dataset.s = "now";
            const p = reduceMotion ? 1 : Math.min(Math.max((posMs - ws) / (we - ws), 0), 1);
            span.style.setProperty("--p", p.toFixed(3));
          } else if (span.dataset.s !== "next") {
            span.dataset.s = "next";
          }
        }
      }

      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [synced, lines, offsetMs]);

  const onSeek = useCallback(
    (line: LyricLine) => {
      if (synced && line.timeMs != null && seekable) seek(line.timeMs / 1000);
    },
    [synced, seekable, seek],
  );

  if (!meta) return <Centered>Nothing playing.</Centered>;
  if (loading) {
    return (
      <Centered>
        <Loader2 className="size-5 animate-spin text-text-faint" aria-hidden="true" />
        <span className="mt-2">Finding lyrics…</span>
      </Centered>
    );
  }
  if (!lines || lines.length === 0) {
    return (
      <Centered>
        <MicVocal className="size-7 text-text-faint" aria-hidden="true" />
        <span className="mt-2">No lyrics found</span>
        <span className="mt-1 max-w-[14rem] text-xs text-text-faint">
          for “{meta.title}”. Drop a matching .lrc file next to the track, or it
          may not be on LRCLIB yet.
        </span>
      </Centered>
    );
  }

  // Plain (unsynced) lyrics: readable, scrollable, no animation.
  if (!synced) {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
        {lines.map((line, i) => (
          <p key={i} className="py-1.5 text-[15px] leading-relaxed text-text-muted">
            {line.text || "♪"}
          </p>
        ))}
        <div className="h-16" aria-hidden="true" />
      </div>
    );
  }

  return (
    <div className="relative min-h-0 flex-1 overflow-hidden">
      <WordStyles />
      {/* Blurred album backdrop — the lyrics float above it, Apple-style. */}
      {meta.cover && (
        <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden="true">
          <div
            className="absolute inset-0 scale-150 opacity-30 blur-3xl saturate-150"
            style={{
              backgroundImage: `url(${meta.cover})`,
              backgroundSize: "cover",
              backgroundPosition: "center",
            }}
          />
          <div className="absolute inset-0 bg-surface/70" />
        </div>
      )}
      <div
        ref={viewportRef}
        className="relative h-full overflow-hidden [mask-image:linear-gradient(to_bottom,transparent,#000_16%,#000_82%,transparent)]"
      >
        <div
          ref={listRef}
          className="px-5 will-change-transform"
          style={{ paddingTop: focal, paddingBottom: viewportH - focal }}
        >
          {lines.map((line, i) => {
            const d = active < 0 ? 99 : Math.abs(i - active);
            const isActive = i === active;
            const canSeek = line.timeMs != null && seekable;
            if (isActive) {
              return (
                <p
                  key={i}
                  ref={activeRef}
                  data-idx={i}
                  onClick={canSeek ? () => onSeek(line) : undefined}
                  aria-current="true"
                  style={{ textShadow: "0 0 22px rgba(255,255,255,0.10)" }}
                  className={cn(
                    "origin-left py-2.5 text-[26px] font-bold leading-[1.18] tracking-tight",
                    "transition-transform duration-500 ease-out will-change-transform",
                    canSeek && "cursor-pointer",
                    reduceMotion ? "scale-100 text-text" : "scale-[1.03]",
                  )}
                >
                  {reduceMotion || !activeWords ? (
                    <span className="text-text">{line.text || "♪"}</span>
                  ) : (
                    activeWords.map((w, k) => (
                      <span
                        key={k}
                        data-w
                        data-s="next"
                        data-start={w.startMs}
                        data-end={w.endMs}
                        className="am-word"
                      >
                        {w.text}
                        {k < activeWords.length - 1 ? " " : ""}
                      </span>
                    ))
                  )}
                </p>
              );
            }
            return (
              <p
                key={i}
                onClick={canSeek ? () => onSeek(line) : undefined}
                style={{
                  filter: reduceMotion ? undefined : `blur(${Math.min(d, 4) * 0.7}px)`,
                  opacity: d === 1 ? 0.6 : d === 2 ? 0.42 : i < active ? 0.22 : 0.3,
                }}
                className={cn(
                  "origin-left py-2 text-[22px] font-semibold leading-[1.18] tracking-tight text-text",
                  "transition-[opacity,filter,transform] duration-500 ease-out",
                  canSeek && "cursor-pointer hover:opacity-90",
                )}
              >
                {line.text || "♪"}
              </p>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** Word-fill styling — a left-to-right gradient sweep across the current word. */
function WordStyles() {
  return (
    <style>{`
      .am-word {
        color: rgba(255,255,255,0.28);
        transition: color .28s cubic-bezier(.4,0,.2,1);
      }
      .am-word[data-s="past"] { color: #fff; }
      .am-word[data-s="now"] {
        color: transparent;
        background-image: linear-gradient(90deg,
          #fff calc(var(--p,0) * 100%),
          rgba(255,255,255,0.28) calc(var(--p,0) * 100%));
        -webkit-background-clip: text;
        background-clip: text;
      }
    `}</style>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-4 text-center text-sm text-text-muted">
      {children}
    </div>
  );
}
