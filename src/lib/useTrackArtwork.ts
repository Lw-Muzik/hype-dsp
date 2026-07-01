import { useEffect, useState } from "react";
import { cloudTrackCover, libraryArtwork, linkArtwork } from "@/lib/ipc";

/**
 * Where a track's cover art comes from. Local files read embedded art by path;
 * phone tracks fetch it from the paired device; cloud tracks fetch it lazily
 * from the metadata cache (only the on-screen rows, so a big library never holds
 * thousands of covers in memory).
 */
export interface ArtSource {
  /** Stable cache key (the track's uid). */
  key: string;
  source: "local" | "phone" | "cloud";
  /** Local file path. */
  path?: string | null;
  /** Phone device + track ids. */
  deviceId?: string;
  trackId?: string;
  /** Whether the phone reports embedded art (skip the fetch if false). */
  hasArt?: boolean;
  /** Cloud account + file ids + name, to resolve the cover lazily on demand. */
  cloudAccountId?: string;
  cloudFileId?: string;
  cloudName?: string;
  /** Already-resolved cover (a `data:` URI), e.g. the now-playing track after
   *  decode — used directly, skipping any fetch. */
  cover?: string | null;
}

/**
 * Lazily resolve a track's cover art (a `data:` URI), source-aware. Results
 * (including "no art" = `null`) are cached per track for the session and
 * in-flight requests de-duplicated, so scrolling a huge grid/list stays cheap.
 */
const cache = new Map<string, string | null>();
const inFlight = new Map<string, Promise<string | null>>();

// Bound the cache so covers don't accumulate unbounded as you scroll a huge
// library over a session (each is a ~100 KB `data:` URI). A cap well above the
// on-screen working set means back-and-forth scrolling still hits cache; only
// long-gone rows are evicted (oldest-inserted first — a visible row was just
// inserted, so it's never the one dropped). ~400 covers ≈ a few tens of MB max.
const CACHE_CAP = 400;
function cacheSet(key: string, value: string | null): void {
  cache.set(key, value);
  while (cache.size > CACHE_CAP) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

function resolve(art: ArtSource): Promise<string | null> {
  // A cover already in hand (e.g. the now-playing track post-decode) wins.
  if (art.cover) return Promise.resolve(art.cover);
  if (art.source === "local" && art.path) {
    return libraryArtwork(art.path);
  }
  if (art.source === "phone" && art.hasArt && art.deviceId && art.trackId) {
    return linkArtwork(art.deviceId, art.trackId);
  }
  if (art.source === "cloud" && art.cloudAccountId && art.cloudFileId) {
    return cloudTrackCover(
      art.cloudAccountId,
      art.cloudFileId,
      art.cloudName ?? "",
    );
  }
  return Promise.resolve(null);
}

function fetchArtwork(art: ArtSource): Promise<string | null> {
  const existing = inFlight.get(art.key);
  if (existing) return existing;
  const p = resolve(art)
    .then((v) => v ?? null)
    .catch(() => null)
    .then((v) => {
      cacheSet(art.key, v);
      inFlight.delete(art.key);
      return v;
    });
  inFlight.set(art.key, p);
  return p;
}

/** The track's cover art data URI, or `null` while loading / if it has none. */
export function useTrackArtwork(art: ArtSource | null | undefined): string | null {
  const key = art?.key ?? null;
  const [cover, setCover] = useState<string | null>(() =>
    key ? (cache.get(key) ?? null) : null,
  );

  useEffect(() => {
    if (!art || !key) {
      setCover(null);
      return;
    }
    if (cache.has(key)) {
      setCover(cache.get(key)!);
      return;
    }
    // Defer slightly: rows scrolled straight past unmount before this fires, so
    // a fast scroll never kicks off hundreds of fetches. Cached/in-flight keys
    // resolve immediately via the dedup above.
    let active = true;
    const timer = window.setTimeout(() => {
      void fetchArtwork(art).then((v) => {
        if (active) setCover(v);
      });
    }, 140);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
    // Only re-resolve when the track changes (art content is stable per key).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  return cover;
}
