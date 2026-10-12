/**
 * Translations between the protocol's track/catalog shapes and the app's own
 * models. Pure functions — the bridge, the RPC handlers and the state
 * publisher all lean on these, and the tests pin them directly.
 */
import type { ExploreItem, ExploreShelf, LibraryTrack, YtTrack } from "@/lib/types";
import type { QueueItem } from "@/stores/engine";
import type { Item, LocalRef, Shelf, TrackRef, YtRef } from "./protocol";

const MAX_ART_URL = 2048;

/** Only public https art may leave the device: the browser can't load the
 *  `data:` covers cloud/local tracks carry, and they'd bloat every frame. */
export function publicArt(url: string | null | undefined): string | null {
  return typeof url === "string" && url.startsWith("https://") && url.length <= MAX_ART_URL
    ? url
    : null;
}

export function secsToMs(secs: number | null | undefined): number | null {
  return typeof secs === "number" && Number.isFinite(secs) ? Math.max(0, Math.round(secs * 1000)) : null;
}

export function msToSecs(ms: number | null | undefined): number | null {
  return typeof ms === "number" && Number.isFinite(ms) ? Math.max(0, ms) / 1000 : null;
}

/** A `yt` ref as the queue's YouTube Music track. Shaped like a search result
 *  (`track_of` in hm-ytmusic): a lone song is its own "playlist". */
export function ytRefToTrack(ref: YtRef): YtTrack {
  return {
    videoId: ref.videoId,
    title: ref.title,
    artist: ref.artist,
    album: ref.album,
    durationSecs: msToSecs(ref.durationMs),
    thumbnail: ref.artUrl,
    playlistId: ref.videoId,
    playlistTitle: ref.title,
    isAvailable: true,
    // A ref doesn't say; video stays opt-in from the Video tab as usual.
    hasVideo: false,
  };
}

export function ytTrackToRef(t: YtTrack): YtRef {
  return {
    src: "yt",
    videoId: t.videoId,
    title: t.title,
    artist: t.artist,
    album: t.album,
    durationMs: secsToMs(t.durationSecs),
    artUrl: publicArt(t.thumbnail),
  };
}

export function libraryTrackToRef(t: LibraryTrack, deviceId: string): LocalRef {
  return {
    src: "local",
    deviceId,
    id: t.path,
    title: t.title,
    artist: t.artist,
    album: t.album,
    durationMs: secsToMs(t.durationSecs),
    artUrl: null,
  };
}

/** What a controller is told about a queue item. Only YouTube Music and this
 *  desktop's own files are playable elsewhere; everything else (cloud, phone,
 *  radio, a phone's cast) is `other`. */
export function queueItemToRef(item: QueueItem, deviceId: string): TrackRef {
  const durationMs = secsToMs(item.durationSecs);
  if (item.source === "ytmusic" && item.ytTrack) {
    return {
      src: "yt",
      videoId: item.ytTrack.videoId,
      title: item.title,
      artist: item.artist,
      album: item.album,
      durationMs,
      artUrl: publicArt(item.cover ?? item.ytTrack.thumbnail),
    };
  }
  if (item.source === "local" && item.track) {
    return {
      src: "local",
      deviceId,
      id: item.track.path,
      title: item.title,
      artist: item.artist,
      album: item.album,
      durationMs,
      artUrl: null,
    };
  }
  return {
    src: "other",
    title: item.title,
    artist: item.artist,
    album: item.album,
    durationMs,
    artUrl: publicArt(item.cover),
  };
}

export function exploreItemToItem(i: ExploreItem): Item {
  return {
    kind: i.kind,
    id: i.id,
    title: i.title,
    subtitle: i.subtitle ?? null,
    thumbnail: i.thumbnail ?? null,
    artist: i.artist ?? null,
    album: i.album ?? null,
    durationMs: secsToMs(i.durationSecs),
    hasVideo: i.hasVideo ?? false,
  };
}

export function itemToExploreItem(i: Item): ExploreItem {
  return {
    kind: i.kind,
    id: i.id,
    title: i.title,
    subtitle: i.subtitle,
    thumbnail: i.thumbnail,
    artist: i.artist,
    album: i.album,
    durationSecs: msToSecs(i.durationMs),
    hasVideo: i.hasVideo,
  };
}

export function exploreShelfToShelf(s: ExploreShelf): Shelf {
  return { title: s.title, items: s.items.map(exploreItemToItem) };
}
