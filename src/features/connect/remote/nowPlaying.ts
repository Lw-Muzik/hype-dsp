/**
 * ONE answer to "what is playing now": this computer's engine, or - while the
 * app is a Hype Connect remote - the controlled device's track. Surfaces that
 * show the current song (ambient backdrop, sidebar card, tray) read this
 * instead of the engine store directly, so none of them needs its own remote
 * check.
 */
import type { TrackMeta } from "@/lib/types";
import { useEngineStore } from "@/stores/engine";
import { positionNow, type RemoteView } from "./session";
import { useRemoteStore } from "./store";

export interface NowPlaying {
  /** True when the track is playing on another device. */
  remote: boolean;
  /** The other device's name (remote mode only). */
  deviceName: string | null;
  /** Title/artist/album/art; `cover` is null when the source has none. */
  meta: TrackMeta | null;
}

/** A remote track as the shared `TrackMeta` (art = the yt thumbnail, if any). */
export function remoteMeta(view: RemoteView): TrackMeta | null {
  const t = view.track;
  if (!t) return null;
  return { title: t.title, artist: t.artist, album: t.album, cover: t.artUrl ?? null };
}

/** The current song: the remote's while one is controlled, else the local one. */
export function selectNowPlaying(localMeta: TrackMeta | null, remote: RemoteView | null): NowPlaying {
  if (remote) return { remote: true, deviceName: remote.device.name, meta: remoteMeta(remote) };
  return { remote: false, deviceName: null, meta: localMeta };
}

/** Progress in seconds (`duration` 0 = unknown) for the current song. */
export interface NowProgress {
  position: number;
  duration: number;
}

/** The remote's progress at hub time `hubNowMs`: `positionMs + (playing ? hubNow - at : 0)`, clamped (§3.6). */
export function remoteProgress(view: RemoteView, hubNowMs: number): NowProgress {
  const durationMs = view.durationMs ?? view.track?.durationMs ?? 0;
  const clamped = positionNow({ ...view, durationMs: durationMs > 0 ? durationMs : view.durationMs }, hubNowMs);
  return { position: clamped / 1000, duration: durationMs / 1000 };
}

export function useNowPlaying(): NowPlaying {
  const localMeta = useEngineStore((s) => s.nowPlayingMeta);
  const remote = useRemoteStore((s) => s.view);
  return selectNowPlaying(localMeta, remote);
}
