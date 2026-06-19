import { useCallback, useEffect, useMemo, useState } from "react";
import {
  cloudAllAudio,
  cloudStatus,
  cloudTrackMetadata,
  libraryList,
  linkLibrary,
  linkPaired,
} from "@/lib/ipc";
import { cloudItem, localItem, phoneItem } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import type { ArtSource } from "@/lib/useTrackArtwork";
import type { CloudProvider, CloudTrackMeta } from "@/lib/types";

/** A browsable track from any source, ready to enqueue (it's a `QueueItem`). */
export interface MusicTrack extends QueueItem {
  source: "local" | "phone" | "cloud";
  /** Unique across sources, for React keys and highlight matching. */
  uid: string;
  /** Genre from tags (local only for now), for the Genres facet. */
  genre: string | null;
  /** Folder/grouping label for the Folders facet. */
  folder: string | null;
  /** Local file path for lazy embedded artwork (null for phone/cloud). */
  artPath: string | null;
  /** Pre-resolved cover (a `data:` URI) — cloud tracks after metadata preload. */
  cover: string | null;
}

/** Per-source availability + counts, for the source filter UI. */
export interface SourceState {
  /** Whether the source is reachable/connected (library is always true). */
  connected: boolean;
  loading: boolean;
  count: number;
}

export interface MusicLibrary {
  tracks: MusicTrack[];
  library: SourceState;
  phone: SourceState;
  cloud: SourceState;
  /** Re-scan every source (e.g. after pairing a phone or connecting cloud). */
  reload: () => void;
}

/** Where to resolve a track's cover art from, by source (local path / phone). */
export function trackArt(t: MusicTrack): ArtSource {
  if (t.source === "phone" && t.device && t.phoneTrack) {
    return {
      key: t.uid,
      source: "phone",
      deviceId: t.device.id,
      trackId: t.phoneTrack.id,
      hasArt: t.phoneTrack.hasArt,
    };
  }
  if (t.source === "cloud") {
    return { key: t.uid, source: "cloud", cover: t.cover };
  }
  return { key: t.uid, source: "local", path: t.artPath };
}

/** The immediate parent folder name of a file path (for the Folders facet). */
function parentFolder(path: string): string | null {
  const norm = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = norm.split("/").filter(Boolean);
  return parts.length >= 2 ? parts[parts.length - 2]! : null;
}

/** Collect a connected provider's audio files in one flat, account-wide listing
 *  (all folders) — mirrors the mobile app, so songs nested in subfolders are
 *  included rather than truncated by a bounded folder walk. */
async function scanCloud(provider: CloudProvider): Promise<MusicTrack[]> {
  let entries;
  try {
    entries = await cloudAllAudio(provider);
  } catch {
    return [];
  }
  return entries.map((e) => ({
    ...cloudItem(e),
    source: "cloud" as const,
    uid: `cloud:${provider}:${e.id}`,
    genre: null,
    folder: e.folder ?? null,
    artPath: null,
    cover: null,
  }));
}

// How many cloud files to read metadata for at once (the backend caches each
// after the first read, so this only bites on the first scan).
const CLOUD_META_CONCURRENCY = 4;

/**
 * Read embedded tags (title/artist/album + cover) for cloud tracks in the
 * background, like the mobile app's `CloudMetadataService`. Runs a few at a time
 * and reports each result through `onResult`; `isStale` lets the caller cancel
 * (e.g. on disconnect / reload). Backend-cached, so re-scans are cheap.
 */
async function preloadCloudMeta(
  tracks: MusicTrack[],
  onResult: (uid: string, meta: CloudTrackMeta) => void,
  isStale: () => boolean,
): Promise<void> {
  let next = 0;
  const worker = async () => {
    while (next < tracks.length && !isStale()) {
      const t = tracks[next++];
      const file = t?.cloud;
      if (!t || !file) continue;
      try {
        const meta = await cloudTrackMetadata(file.provider, file.id, file.name);
        if (meta && !isStale()) onResult(t.uid, meta);
      } catch {
        // Skip — a single failed read shouldn't stop the rest.
      }
    }
  };
  await Promise.all(
    Array.from({ length: CLOUD_META_CONCURRENCY }, () => worker()),
  );
}

/**
 * Aggregates every reachable source — the local library, paired phones, and
 * connected cloud accounts — into one collection of browsable tracks. Each
 * source loads independently and resiliently (a failure contributes nothing
 * rather than breaking the whole library), so the unified browse + search work
 * across whatever is currently available.
 */
export function useMusicLibrary(): MusicLibrary {
  const libraryVersion = useLibraryStore((s) => s.version);

  const [local, setLocal] = useState<MusicTrack[]>([]);
  const [phone, setPhone] = useState<MusicTrack[]>([]);
  // Cloud tracks as listed (filename titles), plus tags resolved lazily in the
  // background and merged on top by uid.
  const [cloudBase, setCloudBase] = useState<MusicTrack[]>([]);
  const [cloudMeta, setCloudMeta] = useState<Map<string, CloudTrackMeta>>(
    () => new Map(),
  );
  const [libLoading, setLibLoading] = useState(true);
  const [phoneState, setPhoneState] = useState<SourceState>({
    connected: false,
    loading: false,
    count: 0,
  });
  const [cloudState, setCloudState] = useState<SourceState>({
    connected: false,
    loading: false,
    count: 0,
  });
  const [nonce, setNonce] = useState(0);
  const reload = useCallback(() => setNonce((n) => n + 1), []);

  // Local library (always present).
  useEffect(() => {
    let cancelled = false;
    setLibLoading(true);
    libraryList()
      .then((tracks) => {
        if (cancelled) return;
        setLocal(
          tracks.map((t) => ({
            ...localItem(t),
            source: "local" as const,
            uid: `local:${t.path}`,
            genre: t.genre,
            folder: parentFolder(t.path),
            artPath: t.path,
            cover: null,
          })),
        );
      })
      .catch(() => !cancelled && setLocal([]))
      .finally(() => !cancelled && setLibLoading(false));
    return () => {
      cancelled = true;
    };
  }, [libraryVersion, nonce]);

  // Paired phones (each may be offline — skipped gracefully).
  useEffect(() => {
    let cancelled = false;
    setPhoneState((s) => ({ ...s, loading: true }));
    linkPaired()
      .then(async (devices) => {
        if (cancelled) return;
        const lists = await Promise.all(
          devices.map((d) =>
            linkLibrary(d.id)
              .then((tracks) =>
                tracks.map((t) => ({
                  ...phoneItem(d, t),
                  source: "phone" as const,
                  uid: `phone:${d.id}:${t.id}`,
                  genre: null,
                  folder: d.name,
                  artPath: null,
                  cover: null,
                })),
              )
              .catch(() => [] as MusicTrack[]),
          ),
        );
        if (cancelled) return;
        const merged = lists.flat();
        setPhone(merged);
        setPhoneState({
          connected: devices.length > 0,
          loading: false,
          count: merged.length,
        });
      })
      .catch(() => {
        if (cancelled) return;
        setPhone([]);
        setPhoneState({ connected: false, loading: false, count: 0 });
      });
    return () => {
      cancelled = true;
    };
  }, [nonce]);

  // Connected cloud accounts (flat account-wide listing).
  useEffect(() => {
    let cancelled = false;
    setCloudState((s) => ({ ...s, loading: true }));
    setCloudMeta(new Map());
    cloudStatus()
      .then(async (status) => {
        if (cancelled) return;
        const connected = [
          status.googleConnected ? ("googleDrive" as const) : null,
          status.dropboxConnected ? ("dropbox" as const) : null,
        ].filter((p): p is CloudProvider => p !== null);
        if (connected.length === 0) {
          setCloudBase([]);
          setCloudState({ connected: false, loading: false, count: 0 });
          return;
        }
        const lists = await Promise.all(connected.map(scanCloud));
        if (cancelled) return;
        const merged = lists.flat();
        setCloudBase(merged);
        setCloudState({ connected: true, loading: false, count: merged.length });
      })
      .catch(() => {
        if (cancelled) return;
        setCloudBase([]);
        setCloudState({ connected: false, loading: false, count: 0 });
      });
    return () => {
      cancelled = true;
    };
  }, [nonce]);

  // Resolve cloud tags (title/artist/album + cover) in the background, merging
  // each result in by uid as it arrives.
  useEffect(() => {
    if (cloudBase.length === 0) return;
    let stale = false;
    void preloadCloudMeta(
      cloudBase,
      (uid, meta) =>
        setCloudMeta((prev) => {
          const next = new Map(prev);
          next.set(uid, meta);
          return next;
        }),
      () => stale,
    );
    return () => {
      stale = true;
    };
  }, [cloudBase]);

  // Cloud tracks with resolved tags merged over the listed filenames.
  const cloud = useMemo(
    () =>
      cloudMeta.size === 0
        ? cloudBase
        : cloudBase.map((t) => {
            const m = cloudMeta.get(t.uid);
            if (!m) return t;
            return {
              ...t,
              title: m.title ?? t.title,
              artist: m.artist ?? t.artist,
              album: m.album ?? t.album,
              cover: m.cover ?? t.cover,
            };
          }),
    [cloudBase, cloudMeta],
  );

  const tracks = useMemo(() => [...local, ...phone, ...cloud], [local, phone, cloud]);

  return {
    tracks,
    library: { connected: true, loading: libLoading, count: local.length },
    phone: phoneState,
    cloud: cloudState,
    reload,
  };
}
