/**
 * Remote catalog calls (§3.8), answered through the same IPC the Explore and
 * Library views use: `search`, `suggest`, `open` and `artist` run on this
 * desktop's own YouTube Music session; `library` pages its local files;
 * `lyrics` (v1.1) reads what the Lyrics panel would show.
 */
import type { ExploreItem, ExploreShelf, LibraryTrack, YtTrack } from "@/lib/types";
import {
  decodeItem,
  tryDecode,
  type RpcError,
  type RpcMethod,
  type SearchFilter,
} from "./protocol";
import { answerLyrics, type LyricsPort } from "./lyrics";
import { exploreShelfToShelf, itemToExploreItem, libraryTrackToRef, ytTrackToRef } from "./refs";

export interface RpcPort {
  /** This desktop's Connect device id (its install id). */
  deviceId(): string;
  /** Whether a YouTube Music session exists (may be cached briefly). */
  ytSignedIn(): Promise<boolean>;
  search(query: string, filter: string): Promise<ExploreShelf[]>;
  suggest(query: string): Promise<string[]>;
  exploreTracks(item: ExploreItem): Promise<YtTrack[]>;
  artistPage(browseId: string): Promise<ExploreShelf[]>;
  /** The local library, in the order the Library shows it. */
  localTracks(): Promise<LibraryTrack[]>;
  /** A YT call failed for want of a session: forget any cached sign-in. */
  forgetSignIn(): void;
  /** The playing track's lyrics, via the app's own resolver. */
  lyrics: LyricsPort;
}

export type RpcOutcome =
  | { ok: true; result: Record<string, unknown> }
  | { ok: false; error: RpcError };

const fail = (error: RpcError): RpcOutcome => ({ ok: false, error });
const done = (result: Record<string, unknown>): RpcOutcome => ({ ok: true, result });

/** The contract's filters → the app's (`top` is what YouTube calls "all"). */
const FILTERS: Record<SearchFilter, string> = {
  all: "top",
  songs: "songs",
  videos: "videos",
  albums: "albums",
  artists: "artists",
  playlists: "playlists",
};

export const MAX_LIBRARY_PAGE = 200;

/** hm-ytmusic reports a missing session as "Not signed in to YouTube Music." */
function isSignedOutError(e: unknown): boolean {
  const message =
    typeof e === "object" && e !== null && "message" in e
      ? String((e as { message: unknown }).message)
      : String(e);
  return /not signed in/i.test(message);
}

function stringParam(params: Record<string, unknown>, key: string): string | null {
  const v = params[key];
  return typeof v === "string" ? v : null;
}

function intParam(params: Record<string, unknown>, key: string): number | null {
  const v = params[key];
  return typeof v === "number" && Number.isFinite(v) ? Math.max(0, Math.floor(v)) : null;
}

function matches(t: LibraryTrack, needle: string): boolean {
  return [t.title, t.artist, t.album].some((f) => f?.toLowerCase().includes(needle));
}

/** Answer one rpc. Never throws: a failure is an `rpc_result` error. */
export async function executeRpc(
  method: RpcMethod,
  params: Record<string, unknown>,
  port: RpcPort,
): Promise<RpcOutcome> {
  try {
    if (method === "library") return await library(params, port);
    // Lyrics come from the app's own resolver, not from YouTube Music.
    if (method === "lyrics") return done({ ...(await answerLyrics(port.lyrics)) });
    if (!(await port.ytSignedIn())) return fail("not_signed_in");
    return await youtube(method, params, port);
  } catch (e) {
    if (isSignedOutError(e)) {
      port.forgetSignIn();
      return fail("not_signed_in");
    }
    return fail("failed");
  }
}

async function youtube(
  method: Exclude<RpcMethod, "library" | "lyrics">,
  params: Record<string, unknown>,
  port: RpcPort,
): Promise<RpcOutcome> {
  switch (method) {
    case "search": {
      const query = stringParam(params, "query");
      const filter = stringParam(params, "filter") as SearchFilter | null;
      if (query === null || filter === null || !(filter in FILTERS)) return fail("failed");
      const shelves = await port.search(query, FILTERS[filter]);
      return done({ shelves: shelves.map(exploreShelfToShelf) });
    }
    case "suggest": {
      const query = stringParam(params, "query");
      if (query === null) return fail("failed");
      return done({ suggestions: await port.suggest(query) });
    }
    case "open": {
      const item = tryDecode(decodeItem, params.item);
      if (!item) return fail("failed");
      // Unavailable tracks can't stream here, so they aren't offered either —
      // the same rule the Explore view applies before queueing.
      const tracks = (await port.exploreTracks(itemToExploreItem(item))).filter((t) => t.isAvailable);
      return done({ title: item.title, tracks: tracks.map(ytTrackToRef) });
    }
    case "artist": {
      const id = stringParam(params, "id");
      if (id === null) return fail("failed");
      const shelves = await port.artistPage(id);
      return done({ shelves: shelves.map(exploreShelfToShelf) });
    }
  }
}

async function library(params: Record<string, unknown>, port: RpcPort): Promise<RpcOutcome> {
  const offset = intParam(params, "offset");
  const limit = intParam(params, "limit");
  if (offset === null || limit === null) return fail("failed");
  const query = stringParam(params, "query")?.trim().toLowerCase() ?? "";
  const all = await port.localTracks();
  const hits = query ? all.filter((t) => matches(t, query)) : all;
  const page = hits.slice(offset, offset + Math.min(limit, MAX_LIBRARY_PAGE));
  const deviceId = port.deviceId();
  return done({
    total: hits.length,
    tracks: page.map((t) => libraryTrackToRef(t, deviceId)),
  });
}
