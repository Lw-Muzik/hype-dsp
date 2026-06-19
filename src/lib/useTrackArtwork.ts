import { useEffect, useState } from "react";
import { libraryArtwork, linkArtwork } from "@/lib/ipc";

/**
 * Where a track's cover art comes from. Local files read embedded art by path;
 * phone tracks fetch it from the paired device; cloud tracks have no per-track
 * art endpoint (they fall back to a gradient).
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
  /** Already-resolved cover (a `data:` URI), e.g. from the cloud metadata
   *  preload — used directly, skipping any fetch. */
  cover?: string | null;
}

/**
 * Lazily resolve a track's cover art (a `data:` URI), source-aware. Results
 * (including "no art" = `null`) are cached per track for the session and
 * in-flight requests de-duplicated, so scrolling a huge grid/list stays cheap.
 */
const cache = new Map<string, string | null>();
const inFlight = new Map<string, Promise<string | null>>();

function resolve(art: ArtSource): Promise<string | null> {
  if (art.source === "local" && art.path) {
    return libraryArtwork(art.path);
  }
  if (art.source === "phone" && art.hasArt && art.deviceId && art.trackId) {
    return linkArtwork(art.deviceId, art.trackId);
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
      cache.set(art.key, v);
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
