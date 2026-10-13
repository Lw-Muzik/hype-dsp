/**
 * The real `PlayerPort` / `RpcPort`: the engine store, the music library
 * store and the YouTube Music IPC — the same calls the app's own views make.
 */
import {
  ytmusicArtistPage,
  ytmusicExploreTracks,
  ytmusicSearch,
  ytmusicSearchSuggestions,
  ytmusicStatus,
} from "@/lib/ipc";
import type { LibraryTrack } from "@/lib/types";
import { useEngineStore } from "@/stores/engine";
import { useLibraryStore } from "@/stores/library";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import { lookupLyrics } from "@/features/player/useLyrics";
import type { PlayerPort } from "./commands";
import type { LyricsPort } from "./lyrics";
import { queueItemToRef } from "./refs";
import type { RpcPort } from "./rpc";
import { activeQueueItem } from "./statePublisher";

/** The `getState`/`subscribe` half of a zustand store. */
export interface StoreLike<T> {
  getState(): T;
  subscribe(listener: (state: T) => void): () => void;
}

/** Resolve once `done(state)` holds, or after `timeoutMs` either way. */
export function waitForStore<T>(
  store: StoreLike<T>,
  done: (state: T) => boolean,
  timeoutMs: number,
): Promise<void> {
  if (done(store.getState())) return Promise.resolve();
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      unsubscribe();
      resolve();
    };
    const timer = setTimeout(finish, timeoutMs);
    const unsubscribe = store.subscribe((s) => {
      if (done(s)) finish();
    });
  });
}

const LIBRARY_WAIT_MS = 6_000;

/** The local library, loading it first if nothing has asked for it yet (the
 *  Library view loads it lazily). Waits a bounded time: a huge first load
 *  answers with what has streamed in so far rather than outlast the hub's
 *  command/rpc timeout. */
export async function localLibrary(): Promise<LibraryTrack[]> {
  const lib = useMusicLibraryStore.getState();
  if (lib.localLoad === "idle") lib.ensureLocal(useLibraryStore.getState().version);
  await waitForStore(useMusicLibraryStore, (s) => s.localLoad !== "loading", LIBRARY_WAIT_MS);
  return useMusicLibraryStore
    .getState()
    .local.flatMap((t) => (t.track ? [t.track] : []));
}

interface SeekView {
  queueIndex: number;
  playing: boolean;
  seekable: boolean;
  positionSecs: number;
  seek: (secs: number) => void;
}

/** Seek to `secs` once the track that is current *now* is actually playing
 *  and seekable (a stream only becomes seekable once it's open). Gives up if
 *  the track changes or nothing happens within `timeoutMs`. */
export function seekWhenReady(store: StoreLike<SeekView>, secs: number, timeoutMs = 20_000): void {
  const track = store.getState().queueIndex;
  let finished = false;
  let unsubscribe = () => {};
  const finish = () => {
    finished = true;
    clearTimeout(timer);
    unsubscribe();
  };
  const timer = setTimeout(finish, timeoutMs);
  const check = (s: SeekView) => {
    if (finished) return;
    if (s.queueIndex !== track) return finish();
    // A first progress tick (position > 0) means the engine has the source
    // open; seeking before that can land on the previous track.
    if (s.playing && s.seekable && s.positionSecs > 0) {
      finish();
      s.seek(secs);
    }
  };
  unsubscribe = store.subscribe(check);
  check(store.getState());
}

export function enginePlayerPort(deviceId: () => string): PlayerPort {
  const engine = () => useEngineStore.getState();
  return {
    deviceId,
    findLocal: async (path) => (await localLibrary()).find((t) => t.path === path) ?? null,
    snapshot: () => {
      const s = engine();
      return {
        playing: s.playing,
        paused: s.paused,
        seekable: s.seekable,
        queue: s.queue,
        order: s.order,
        orderPos: s.orderPos,
      };
    },
    playItems: (items, index) => engine().playQueueItems(items, index),
    playRadio: (seed) => engine().playYtRadio(seed),
    startAt: (secs) => seekWhenReady(useEngineStore, secs),
    enqueue: (items, mode) => engine().enqueueItems(items, mode),
    // The media-key paths: "pause"/"play" are explicit states, not toggles.
    pause: () => engine().handleMediaCommand("pause", null),
    resume: () => engine().handleMediaCommand("play", null),
    jumpTo: (pos) => engine().jumpTo(pos),
    next: () => engine().next(),
    prev: () => engine().prev(),
    seek: (secs) => engine().seek(secs),
    stop: () => engine().stop(),
    setVolume: (v) => engine().setMasterVolume(v),
    setShuffle: (on) => engine().setShuffle(on),
    setRepeat: (mode) => engine().setRepeat(mode),
  };
}

/** The Lyrics panel's own resolver and cache, aimed at the track playing now
 *  (`useLyrics` keys it the same way, so a lookup done for one serves the other). */
export function appLyricsPort(deviceId: () => string): LyricsPort {
  return {
    current: () => {
      const s = useEngineStore.getState();
      const item = activeQueueItem(s);
      if (!item) return null;
      return {
        track: queueItemToRef(item, deviceId()),
        key: `${item.source}:${item.id}`,
        title: s.nowPlayingMeta?.title ?? null,
        artist: s.nowPlayingMeta?.artist ?? null,
        durationSecs: item.durationSecs ?? null,
      };
    },
    resolve: (t) =>
      lookupLyrics(
        t.key,
        activeQueueItem(useEngineStore.getState()),
        t.title ?? "",
        t.artist,
        t.durationSecs,
      ),
  };
}

const SIGNED_IN_TTL_MS = 5 * 60_000;
const SIGNED_OUT_TTL_MS = 15_000;

export function appRpcPort(deviceId: () => string): RpcPort {
  // `ytmusic_status` shells out to yt-dlp, so its answer is cached — briefly
  // when signed out, so signing in is noticed almost at once.
  let known: { signedIn: boolean; at: number } | null = null;
  return {
    deviceId,
    ytSignedIn: async () => {
      const now = Date.now();
      if (known && now - known.at < (known.signedIn ? SIGNED_IN_TTL_MS : SIGNED_OUT_TTL_MS)) {
        return known.signedIn;
      }
      const { signedIn } = await ytmusicStatus();
      known = { signedIn, at: now };
      return signedIn;
    },
    forgetSignIn: () => {
      known = null;
    },
    search: ytmusicSearch,
    suggest: ytmusicSearchSuggestions,
    exploreTracks: ytmusicExploreTracks,
    artistPage: ytmusicArtistPage,
    localTracks: localLibrary,
    lyrics: appLyricsPort(deviceId),
  };
}
