import { useEffect, useState } from "react";
import { libraryArtwork } from "@/lib/ipc";

/**
 * Lazily resolve a local track's embedded cover art (a `data:` URI) by path.
 *
 * The library scan skips artwork to stay fast, so covers are fetched on demand
 * for the rows/cards actually rendered. Results — including "no art" (`null`) —
 * are cached per path for the session, and concurrent requests for the same
 * path are de-duplicated, so scrolling a long list stays cheap.
 */
const cache = new Map<string, string | null>();
const inFlight = new Map<string, Promise<string | null>>();

function fetchArtwork(path: string): Promise<string | null> {
  const existing = inFlight.get(path);
  if (existing) return existing;
  const p = libraryArtwork(path)
    .then((v) => v ?? null)
    .catch(() => null)
    .then((v) => {
      cache.set(path, v);
      inFlight.delete(path);
      return v;
    });
  inFlight.set(path, p);
  return p;
}

/** The track's cover art data URI, or `null` while loading / if it has none. */
export function useTrackArtwork(path: string | null | undefined): string | null {
  const [cover, setCover] = useState<string | null>(() =>
    path ? (cache.get(path) ?? null) : null,
  );

  useEffect(() => {
    if (!path) {
      setCover(null);
      return;
    }
    if (cache.has(path)) {
      setCover(cache.get(path)!);
      return;
    }
    // Defer the actual file probe slightly: rows the user scrolls straight past
    // unmount before this fires, so a fast scroll through a huge list never
    // kicks off thousands of disk reads. Already-cached/in-flight paths still
    // resolve immediately via fetchArtwork's dedup.
    let active = true;
    const timer = window.setTimeout(() => {
      void fetchArtwork(path).then((v) => {
        if (active) setCover(v);
      });
    }, 140);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [path]);

  return cover;
}
