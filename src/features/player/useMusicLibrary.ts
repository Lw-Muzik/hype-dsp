import { useCallback, useEffect, useMemo, useState } from "react";
import {
  cloudAllAudio,
  cloudStatus,
  libraryList,
  linkLibrary,
  linkPaired,
} from "@/lib/ipc";
import { cloudItem, localItem, phoneItem } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import type { ArtSource } from "@/lib/useTrackArtwork";
import type { CloudProvider } from "@/lib/types";

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
    return { key: t.uid, source: "cloud" };
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
  }));
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
  const [cloud, setCloud] = useState<MusicTrack[]>([]);
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

  // Connected cloud accounts (bounded recursive scan).
  useEffect(() => {
    let cancelled = false;
    setCloudState((s) => ({ ...s, loading: true }));
    cloudStatus()
      .then(async (status) => {
        if (cancelled) return;
        const connected = [
          status.googleConnected ? ("googleDrive" as const) : null,
          status.dropboxConnected ? ("dropbox" as const) : null,
        ].filter((p): p is CloudProvider => p !== null);
        if (connected.length === 0) {
          setCloud([]);
          setCloudState({ connected: false, loading: false, count: 0 });
          return;
        }
        const lists = await Promise.all(connected.map(scanCloud));
        if (cancelled) return;
        const merged = lists.flat();
        setCloud(merged);
        setCloudState({ connected: true, loading: false, count: merged.length });
      })
      .catch(() => {
        if (cancelled) return;
        setCloud([]);
        setCloudState({ connected: false, loading: false, count: 0 });
      });
    return () => {
      cancelled = true;
    };
  }, [nonce]);

  const tracks = useMemo(() => [...local, ...phone, ...cloud], [local, phone, cloud]);

  return {
    tracks,
    library: { connected: true, loading: libLoading, count: local.length },
    phone: phoneState,
    cloud: cloudState,
    reload,
  };
}
