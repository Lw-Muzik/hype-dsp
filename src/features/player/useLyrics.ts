import { useEffect, useState } from "react";
import { lyricsFetch } from "@/lib/ipc";
import { parseLrc } from "@/lib/lrc";
import type { ParsedLyrics } from "@/lib/lrc";
import { useEngineStore } from "@/stores/engine";

/** Resolved lyrics for the current track (cached per track for the session). */
const cache = new Map<string, ParsedLyrics | null>();
const inFlight = new Map<string, Promise<ParsedLyrics | null>>();

interface LyricsState {
  loading: boolean;
  lyrics: ParsedLyrics | null;
  /** The track key these lyrics belong to (so the view can guard staleness). */
  key: string | null;
}

function fetchFor(
  key: string,
  title: string,
  artist: string | null,
  durationSecs: number | null,
  path: string | null,
): Promise<ParsedLyrics | null> {
  const existing = inFlight.get(key);
  if (existing) return existing;
  const p = lyricsFetch(title, artist, durationSecs, path)
    .then((raw) => (raw ? parseLrc(raw) : null))
    .catch(() => null)
    .then((parsed) => {
      cache.set(key, parsed);
      inFlight.delete(key);
      return parsed;
    });
  inFlight.set(key, p);
  return p;
}

/**
 * Lyrics for whatever is currently playing. Resolves on track change through
 * the backend chain (.lrc / embedded / LRCLIB), parses LRC, and caches by
 * track. The view layers the live position on top for synced highlighting.
 */
export function useLyrics(): LyricsState {
  const meta = useEngineStore((s) => s.nowPlayingMeta);
  const current = useEngineStore((s) =>
    s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined,
  );
  const key = current ? `${current.source}:${current.id}` : null;
  const title = meta?.title ?? null;

  const [state, setState] = useState<LyricsState>({
    loading: false,
    lyrics: null,
    key: null,
  });

  useEffect(() => {
    if (!key || !title) {
      setState({ loading: false, lyrics: null, key: null });
      return;
    }
    if (cache.has(key)) {
      setState({ loading: false, lyrics: cache.get(key)!, key });
      return;
    }
    let active = true;
    setState({ loading: true, lyrics: null, key });
    const path = current?.source === "local" ? (current.track?.path ?? null) : null;
    void fetchFor(key, title, meta?.artist ?? null, current?.durationSecs ?? null, path).then(
      (lyrics) => {
        if (active) setState({ loading: false, lyrics, key });
      },
    );
    return () => {
      active = false;
    };
    // Only re-resolve when the track changes (title/artist/path are stable for it).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, title]);

  return state;
}
