import { useEffect, useState } from "react";
import { linkLyrics, lyricsFetch } from "@/lib/ipc";
import { parseLrc } from "@/lib/lrc";
import type { ParsedLyrics } from "@/lib/lrc";
import { useEngineStore } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";

/** Resolved lyrics for the current track (cached per track for the session).
 *  Bounded so a long session playing thousands of tracks can't grow it without
 *  limit (evicts oldest-inserted; a re-play just re-resolves, which is cheap). */
const cache = new Map<string, ParsedLyrics | null>();
const inFlight = new Map<string, Promise<ParsedLyrics | null>>();
const CACHE_CAP = 300;
function cacheSet(key: string, value: ParsedLyrics | null): void {
  cache.set(key, value);
  while (cache.size > CACHE_CAP) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

/** Forget a track's cached lyrics so they re-resolve (e.g. after its tags are
 *  fixed by identification). Pass no key to clear everything. */
export function clearLyricsCache(key?: string): void {
  if (key) {
    cache.delete(key);
    inFlight.delete(key);
  } else {
    cache.clear();
    inFlight.clear();
  }
}

interface LyricsState {
  loading: boolean;
  lyrics: ParsedLyrics | null;
  /** The track key these lyrics belong to (so the view can guard staleness). */
  key: string | null;
}

/** Resolve raw lyrics for an item, source-aware. A phone track is checked for
 *  its own downloaded `.lrc` first (so the user's sidecars win); local files go
 *  through the backend's sidecar/embedded path; everything then falls back to
 *  the online chain (LRCLIB → HypeMuzik backend) by title/artist. */
async function resolveRaw(
  item: QueueItem | undefined,
  title: string,
  artist: string | null,
  durationSecs: number | null,
): Promise<string | null> {
  if (item?.source === "phone" && item.device) {
    try {
      const phone = await linkLyrics(item.device.id, item.id);
      if (phone) return phone;
    } catch {
      // Phone unreachable — fall through to the online sources.
    }
  }
  const path = item?.source === "local" ? (item.track?.path ?? null) : null;
  return lyricsFetch(title, artist, durationSecs, path);
}

function fetchFor(
  key: string,
  item: QueueItem | undefined,
  title: string,
  artist: string | null,
  durationSecs: number | null,
): Promise<ParsedLyrics | null> {
  const existing = inFlight.get(key);
  if (existing) return existing;
  const p = resolveRaw(item, title, artist, durationSecs)
    .then((raw) => (raw ? parseLrc(raw) : null))
    .catch(() => null)
    .then((parsed) => {
      cacheSet(key, parsed);
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
    void fetchFor(key, current, title, meta?.artist ?? null, current?.durationSecs ?? null).then(
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
