/**
 * LRC lyrics parsing. Handles standard `[mm:ss.xx]` line timestamps (synced),
 * plain unsynced text, `[ar:]/[ti:]/[al:]/[offset:]` metadata tags, and the
 * "Enhanced LRC" `<mm:ss.xx>` per-word timing that drives Apple-Music-style
 * word-by-word highlighting (the full plain text is always kept too, so the UI
 * works whether or not a source provides word timing).
 */
export interface LyricWord {
  /** Milliseconds into the track when this word starts being sung. */
  timeMs: number;
  text: string;
}

export interface LyricLine {
  /** Milliseconds into the track, or null for unsynced lines. */
  timeMs: number | null;
  text: string;
  /** Per-word timing, when the source is Enhanced LRC. */
  words?: LyricWord[];
  /** Start of the next synced line — the moment this one stops being active.
   * Lets the UI pace a line-level fill when there's no word timing. */
  endMs?: number;
}

export interface ParsedLyrics {
  lines: LyricLine[];
  synced: boolean;
  /** True when at least one line carries Enhanced-LRC word timing. */
  wordSynced: boolean;
  /** Tag offset in ms (added to every timestamp), from `[offset:]`. */
  offsetMs: number;
}

const TIMESTAMP = /\[(\d{1,3}):(\d{2})(?:\.(\d{2,3}))?\]/g;
const META = /^\[([a-z]+):(.*)\]$/i;
// Enhanced word-level timing `<mm:ss.xx>`.
const WORD_TIMING = /<\d{1,3}:\d{2}(?:\.\d{2,3})?>/g;
const WORD_TOKEN = /<(\d{1,3}):(\d{2})(?:\.(\d{2,3}))?>/g;

function toMs(min: string, sec: string, frac: string | undefined): number {
  const f = frac ?? "0";
  const ms = f.length === 2 ? parseInt(f, 10) * 10 : parseInt(f, 10);
  return (parseInt(min, 10) * 60 + parseInt(sec, 10)) * 1000 + ms;
}

/** Parse `<mm:ss.xx>word` segments from a line body, or undefined if none. */
function parseWords(body: string): LyricWord[] | undefined {
  const tokens = [...body.matchAll(WORD_TOKEN)];
  if (tokens.length === 0) return undefined;
  const words: LyricWord[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const m = tokens[i]!;
    const from = m.index! + m[0].length;
    const to = i + 1 < tokens.length ? tokens[i + 1]!.index! : body.length;
    const text = body.slice(from, to).replace(WORD_TIMING, "").trimEnd();
    if (text.trim()) {
      words.push({ timeMs: toMs(m[1]!, m[2]!, m[3]), text });
    }
  }
  return words.length ? words : undefined;
}

export function parseLrc(raw: string): ParsedLyrics {
  const lines: LyricLine[] = [];
  let synced = false;
  let wordSynced = false;
  let offsetMs = 0;

  for (const rawLine of raw.split("\n")) {
    const line = rawLine.replace(/\s+$/, "");
    if (!line.trim()) continue;

    const stamps = [...line.matchAll(TIMESTAMP)];

    if (stamps.length === 0) {
      const meta = META.exec(line.trim());
      if (meta) {
        if (meta[1]!.toLowerCase() === "offset") {
          offsetMs = parseInt(meta[2]!.trim(), 10) || 0;
        }
        continue; // metadata tag, not a lyric line
      }
      lines.push({ timeMs: null, text: line.trim() });
      continue;
    }

    // Body = the line minus its `[..]` line stamps; still holds `<..>` words.
    const body = line.replace(TIMESTAMP, "");
    const text = body.replace(WORD_TIMING, "").trim();
    if (!text) continue;
    synced = true;
    // Word times are absolute, so they're only valid for a single occurrence;
    // a repeated-chorus line ([00:01][01:30]…) falls back to line-level fill.
    const words = stamps.length === 1 ? parseWords(body) : undefined;
    if (words) wordSynced = true;
    for (const m of stamps) {
      lines.push({ timeMs: toMs(m[1]!, m[2]!, m[3]), text, words });
    }
  }

  if (synced) {
    lines.sort((a, b) => (a.timeMs ?? Infinity) - (b.timeMs ?? Infinity));
    // Each synced line ends where the next one begins (for line-level pacing).
    for (let i = 0; i < lines.length; i++) {
      if (lines[i]!.timeMs == null) continue;
      const next = lines[i + 1];
      if (next?.timeMs != null) lines[i]!.endMs = next.timeMs;
    }
  }
  return { lines, synced, wordSynced, offsetMs };
}

/** Index of the active line for `positionMs`, or -1 before the first line. */
export function activeLineIndex(
  lines: LyricLine[],
  positionMs: number,
): number {
  let lo = 0;
  let hi = lines.length - 1;
  let result = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const t = lines[mid]!.timeMs;
    if (t == null || t > positionMs) {
      hi = mid - 1;
    } else {
      result = mid;
      lo = mid + 1;
    }
  }
  return result;
}
