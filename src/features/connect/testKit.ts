/** Shared builders for the Hype Connect tests. */
import type { LibraryTrack, YtTrack } from "@/lib/types";
import { localItem, ytmusicItem, type QueueItem } from "@/stores/engine";
import type { LocalRef, YtRef } from "./protocol";

export const DEVICE = "dev-desktop";

export const ytTrack = (videoId: string, over: Partial<YtTrack> = {}): YtTrack => ({
  videoId,
  title: `Song ${videoId}`,
  artist: "Artist",
  album: "Album",
  durationSecs: 200,
  thumbnail: `https://i.ytimg.com/vi/${videoId}/hq.jpg`,
  playlistId: videoId,
  playlistTitle: "x",
  isAvailable: true,
  hasVideo: false,
  ...over,
});

export const libTrack = (path: string, over: Partial<LibraryTrack> = {}): LibraryTrack => ({
  path,
  title: `Local ${path}`,
  artist: "Local Artist",
  album: "Local Album",
  genre: null,
  durationSecs: 180,
  ...over,
});

export const ytItem = (id: string): QueueItem => ytmusicItem(ytTrack(id));
export const localQ = (path: string): QueueItem => localItem(libTrack(path));

export const ytRef = (videoId: string, over: Partial<YtRef> = {}): YtRef => ({
  src: "yt",
  videoId,
  title: `Song ${videoId}`,
  artist: "Artist",
  album: null,
  durationMs: 200_000,
  artUrl: null,
  ...over,
});

export const localRef = (id: string, deviceId = DEVICE, over: Partial<LocalRef> = {}): LocalRef => ({
  src: "local",
  deviceId,
  id,
  title: `Local ${id}`,
  artist: null,
  album: null,
  durationMs: 180_000,
  artUrl: null,
  ...over,
});
