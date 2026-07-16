import { useEffect, useMemo } from "react";
import { useLibraryStore } from "@/stores/library";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import type {
  LoadStatus,
  MusicTrack,
  RecoverableSourceState,
  SourceState,
} from "@/stores/musicLibrary";
import type { ArtSource } from "@/lib/useTrackArtwork";

export type { MusicTrack, RecoverableSourceState, SourceState } from "@/stores/musicLibrary";

export interface MusicLibrary {
  /** Every source merged (local + phone + cloud + YouTube Music). */
  tracks: MusicTrack[];
  /** Per-source track lists, so the browser can switch the source filter
   *  without an O(n) `.filter` over the merged list on every render. */
  localTracks: MusicTrack[];
  phoneTracks: MusicTrack[];
  cloudTracks: MusicTrack[];
  ytmusicTracks: MusicTrack[];
  library: SourceState;
  phone: SourceState;
  cloud: SourceState;
  ytmusic: RecoverableSourceState;
  /** Force every source to reload (e.g. a manual refresh). */
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
    return {
      key: t.uid,
      source: "cloud",
      cloudAccountId: t.cloud?.accountId,
      cloudFileId: t.cloud?.id,
      cloudName: t.cloud?.name,
      cover: t.cover,
    };
  }
  // YT Music thumbnails come with the listing — always a direct URL, never a
  // fetch (a track without one falls back to the gradient).
  if (t.source === "ytmusic") {
    return { key: t.uid, source: "ytmusic", cover: t.cover };
  }
  return { key: t.uid, source: "local", path: t.artPath };
}

/** A finished load (success or error) means connected/count are trustworthy. */
const isReady = (s: LoadStatus) => s === "ready" || s === "error";

/**
 * Aggregates every reachable source — the local library, paired phones,
 * connected cloud accounts, and YouTube Music — into one collection of
 * browsable tracks.
 *
 * State lives in {@link useMusicLibraryStore}, not here, so it survives the
 * Player view unmounting: each source loads **once** and is reused when you
 * return to the Library, reloading only when its data actually changes (a
 * re-scan, a phone pairing, a cloud connect/disconnect). Each source loads
 * independently and resiliently — a failure contributes nothing rather than
 * breaking the whole library.
 */
export function useMusicLibrary(): MusicLibrary {
  const libraryVersion = useLibraryStore((s) => s.version);

  const local = useMusicLibraryStore((s) => s.local);
  const localTotal = useMusicLibraryStore((s) => s.localTotal);
  const phone = useMusicLibraryStore((s) => s.phone);
  const cloudBase = useMusicLibraryStore((s) => s.cloudBase);
  const cloudMeta = useMusicLibraryStore((s) => s.cloudMeta);
  const localLoad = useMusicLibraryStore((s) => s.localLoad);
  const phoneLoad = useMusicLibraryStore((s) => s.phoneLoad);
  const phoneConnected = useMusicLibraryStore((s) => s.phoneConnected);
  const cloudLoad = useMusicLibraryStore((s) => s.cloudLoad);
  const cloudConnected = useMusicLibraryStore((s) => s.cloudConnected);
  const ytmusic = useMusicLibraryStore((s) => s.ytmusic);
  const ytmusicLoad = useMusicLibraryStore((s) => s.ytmusicLoad);
  const ytmusicSignedIn = useMusicLibraryStore((s) => s.ytmusicSignedIn);
  const ytmusicError = useMusicLibraryStore((s) => s.ytmusicError);
  const retryYtMusic = useMusicLibraryStore((s) => s.invalidateYtMusic);
  const ensureLocal = useMusicLibraryStore((s) => s.ensureLocal);
  const ensurePhone = useMusicLibraryStore((s) => s.ensurePhone);
  const ensureCloud = useMusicLibraryStore((s) => s.ensureCloud);
  const ensureYtMusic = useMusicLibraryStore((s) => s.ensureYtMusic);
  const reload = useMusicLibraryStore((s) => s.reloadAll);

  // Kick each source's load-once fetch. Re-runs when the load status flips (so
  // an invalidation back to "idle" reloads), and local also when the library
  // version bumps after a re-scan. The store actions are no-ops otherwise.
  useEffect(() => {
    ensureLocal(libraryVersion);
  }, [ensureLocal, libraryVersion, localLoad]);
  useEffect(() => {
    ensurePhone();
  }, [ensurePhone, phoneLoad]);
  useEffect(() => {
    ensureCloud();
  }, [ensureCloud, cloudLoad]);
  useEffect(() => {
    ensureYtMusic();
  }, [ensureYtMusic, ytmusicLoad]);

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

  const tracks = useMemo(
    () => [...local, ...phone, ...cloud, ...ytmusic],
    [local, phone, cloud, ytmusic],
  );

  return {
    tracks,
    localTracks: local,
    phoneTracks: phone,
    cloudTracks: cloud,
    ytmusicTracks: ytmusic,
    library: {
      connected: true,
      loading: localLoad === "loading",
      ready: isReady(localLoad),
      count: local.length,
      total: Math.max(localTotal, local.length),
    },
    phone: {
      connected: phoneConnected,
      loading: phoneLoad === "loading",
      ready: isReady(phoneLoad),
      count: phone.length,
      total: phone.length,
    },
    cloud: {
      connected: cloudConnected,
      loading: cloudLoad === "loading",
      ready: isReady(cloudLoad),
      count: cloud.length,
      total: cloud.length,
    },
    ytmusic: {
      connected: ytmusicSignedIn,
      loading: ytmusicLoad === "loading",
      ready: isReady(ytmusicLoad),
      count: ytmusic.length,
      total: ytmusic.length,
      error: ytmusicError,
      retry: retryYtMusic,
    },
    reload,
  };
}
