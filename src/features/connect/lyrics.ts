/**
 * The `lyrics` rpc (§3.8, v1.1): a browser shows the lyrics of whatever this
 * desktop is playing, so it asks, and the answer is what the Lyrics panel would
 * show — resolved by the same resolver and cache (`player/useLyrics.ts`), so
 * the two can't disagree.
 *
 * # The 10-second rule
 *
 * The hub gives an rpc 15 s. A lookup can outlast that (fingerprinting a
 * weakly-tagged file, a slow LRCLIB), so after {@link LYRICS_WINDOW_MS} the
 * answer is `found:false` *now* while the lookup carries on; the resolver
 * caches what it finds and the controller's next ask is served from there.
 * Answering late would only buy a `timeout` error.
 */
import type { LyricLine as AppLine, ParsedLyrics } from "@/lib/lrc";
import type { Lyrics, LyricLine, LyricWord, TrackRef } from "./protocol";

export const LYRICS_WINDOW_MS = 10_000;

/** What is playing and what the lookup needs to find its lyrics. */
export interface LyricsTarget {
  /** The controller-facing identity of the track (the one `state` carries). */
  track: TrackRef;
  /** Lyrics-cache key of the track (`source:id`). */
  key: string;
  /** The resolved title; `null` while the stream's metadata is still arriving
   *  — looking up (and caching a miss under) a placeholder title would poison
   *  the lyrics panel's cache for the track. */
  title: string | null;
  artist: string | null;
  durationSecs: number | null;
}

export interface LyricsPort {
  /** The track playing now, or `null` when idle. */
  current(): LyricsTarget | null;
  /** The app's lyrics resolver (cache first); it keeps running if abandoned. */
  resolve(target: LyricsTarget): Promise<ParsedLyrics | null>;
}

/** A line that isn't timed shows for ~4 s in the app's synced view when it has
 *  no successor; the same fallback closes the last word. */
const LAST_LINE_MS = 4_000;

export const noLyrics = (track: TrackRef | null): Lyrics => ({
  track,
  found: false,
  synced: false,
  source: null,
  lines: [],
});

/**
 * The wire form of `parsed` for `track`.
 *
 * * `synced` means *every* line has a start; a mixed list is sent unsynced
 *   (nothing half-timed).
 * * Times are track time: the LRC `[offset:]` the Lyrics panel adds to its
 *   clock (`position - offset >= start`) is applied here, so the controller
 *   needs no knowledge of it.
 * * `words` only when the source really has word timing; each word ends where
 *   the next begins, the last where its line does.
 * * `source` is `null`: the resolver hands back raw text without saying which
 *   of sidecar / embedded / LRCLIB / backend produced it.
 */
export function lyricsToWire(track: TrackRef | null, parsed: ParsedLyrics | null): Lyrics {
  if (!parsed || !parsed.lines.some((l) => l.text.trim() !== "")) return noLyrics(track);
  const synced = parsed.synced && parsed.lines.every((l) => l.timeMs !== null);
  const shift = (ms: number): number => Math.max(0, Math.round(ms + (synced ? parsed.offsetMs : 0)));
  return {
    track,
    found: true,
    synced,
    source: null,
    lines: parsed.lines.map((line): LyricLine => {
      const start = synced && line.timeMs !== null ? shift(line.timeMs) : null;
      return {
        startMs: start,
        text: line.text.trim(),
        words: synced ? wordsToWire(line, shift) : null,
      };
    }),
  };
}

function wordsToWire(line: AppLine, shift: (ms: number) => number): LyricWord[] | null {
  const words = line.words;
  if (!words || words.length === 0 || line.timeMs === null) return null;
  const lineEnd = line.endMs ?? line.timeMs + LAST_LINE_MS;
  return words.map((w, i) => {
    const startMs = shift(w.timeMs);
    const end = shift(words[i + 1]?.timeMs ?? lineEnd);
    return { startMs, endMs: Math.max(startMs, end), text: w.text };
  });
}

/** Resolve to `null` after `ms` without cancelling `p`. */
function withWindow<T>(p: Promise<T>, ms: number): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), ms);
    p.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      () => {
        clearTimeout(timer);
        resolve(null);
      },
    );
  });
}

/** Answer one `lyrics` rpc. Never throws and never waits past `windowMs`. */
export async function answerLyrics(
  port: LyricsPort,
  windowMs: number = LYRICS_WINDOW_MS,
): Promise<Lyrics> {
  const target = port.current();
  if (!target) return noLyrics(null);
  if (target.title === null) return noLyrics(target.track);
  let lookup: Promise<ParsedLyrics | null>;
  try {
    lookup = port.resolve(target);
  } catch {
    return noLyrics(target.track);
  }
  return lyricsToWire(target.track, await withWindow(lookup, windowMs));
}
