import { useEffect, useMemo, useRef } from "react";
import type { ReactNode } from "react";
import { Loader2, MicVocal } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { useLyrics } from "@/features/player/useLyrics";
import { activeLineIndex } from "@/lib/lrc";
import { cn } from "@/lib/cn";

const reduceMotion =
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Lyrics for the current track. Synced LRC highlights and auto-scrolls the
 * active line (tap a line to seek there); plain lyrics render as scrollable
 * text. Resolution + caching live in `useLyrics`; this layers the live position
 * on top.
 */
export function LyricsView() {
  const { loading, lyrics } = useLyrics();
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const positionSecs = useEngineStore((s) => s.positionSecs);
  const seekable = useEngineStore((s) => s.seekable);
  const seek = useEngineStore((s) => s.seek);

  const scrollRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLDivElement>(null);
  // Pause auto-scroll briefly after the user scrolls, so we don't fight them.
  const lastUserScroll = useRef(0);

  const active = useMemo(() => {
    if (!lyrics?.synced) return -1;
    return activeLineIndex(lyrics.lines, positionSecs * 1000 - lyrics.offsetMs);
  }, [lyrics, positionSecs]);

  // Keep the active line centred as it advances.
  useEffect(() => {
    if (active < 0 || !activeRef.current) return;
    if (Date.now() - lastUserScroll.current < 4000) return;
    activeRef.current.scrollIntoView({
      block: "center",
      behavior: reduceMotion ? "auto" : "smooth",
    });
  }, [active]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const mark = () => (lastUserScroll.current = Date.now());
    el.addEventListener("wheel", mark, { passive: true });
    el.addEventListener("pointerdown", mark, { passive: true });
    return () => {
      el.removeEventListener("wheel", mark);
      el.removeEventListener("pointerdown", mark);
    };
  }, []);

  if (!meta) {
    return <Centered>Nothing playing.</Centered>;
  }
  if (loading) {
    return (
      <Centered>
        <Loader2 className="size-5 animate-spin text-text-faint" aria-hidden="true" />
        <span className="mt-2">Finding lyrics…</span>
      </Centered>
    );
  }
  if (!lyrics || lyrics.lines.length === 0) {
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

  return (
    <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
      {lyrics.lines.map((line, i) => {
        const isActive = i === active;
        const canSeek = lyrics.synced && line.timeMs != null && seekable;
        return (
          <p
            key={i}
            ref={isActive ? activeRef : undefined}
            onClick={canSeek ? () => seek(line.timeMs! / 1000) : undefined}
            aria-current={isActive || undefined}
            className={cn(
              "py-1.5 text-[15px] leading-snug transition-colors duration-200",
              canSeek && "cursor-pointer",
              lyrics.synced
                ? isActive
                  ? "font-semibold text-text"
                  : i < active
                    ? "text-text-faint hover:text-text-muted"
                    : "text-text-muted hover:text-text"
                : "text-text-muted",
            )}
          >
            {line.text || "♪"}
          </p>
        );
      })}
      <div className="h-24" aria-hidden="true" />
    </div>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-4 text-center text-sm text-text-muted">
      {children}
    </div>
  );
}
