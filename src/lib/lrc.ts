/**
 * LRC lyrics parsing. Handles standard `[mm:ss.xx]` line timestamps (synced)
 * and plain unsynced text, plus `[ar:]/[ti:]/[al:]/[offset:]` metadata tags.
 * Ported from the mobile app's parser (line-level; word-level timing is
 * stripped to plain text).
 */
export interface LyricLine {
  /** Milliseconds into the track, or null for unsynced lines. */
  timeMs: number | null;
  text: string;
}

export interface ParsedLyrics {
  lines: LyricLine[];
  synced: boolean;
  /** Tag offset in ms (added to every timestamp), from `[offset:]`. */
  offsetMs: number;
}

const TIMESTAMP = /\[(\d{1,3}):(\d{2})(?:\.(\d{2,3}))?\]/g;
const META = /^\[([a-z]+):(.*)\]$/i;
// Enhanced word-level timing `<mm:ss.xx>` — stripped to plain text here.
const WORD_TIMING = /<\d{1,3}:\d{2}(?:\.\d{2,3})?>/g;

function toMs(min: string, sec: string, frac: string | undefined): number {
  const f = frac ?? "0";
  const ms = f.length === 2 ? parseInt(f, 10) * 10 : parseInt(f, 10);
  return (parseInt(min, 10) * 60 + parseInt(sec, 10)) * 1000 + ms;
}

export function parseLrc(raw: string): ParsedLyrics {
  const lines: LyricLine[] = [];
  let synced = false;
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

    const text = line.replace(TIMESTAMP, "").replace(WORD_TIMING, "").trim();
    if (!text) continue;
    synced = true;
    // A line may carry several timestamps ([00:01][00:30]chorus) — one each.
    for (const m of stamps) {
      lines.push({ timeMs: toMs(m[1]!, m[2]!, m[3]), text });
    }
  }

  if (synced) {
    lines.sort((a, b) => (a.timeMs ?? Infinity) - (b.timeMs ?? Infinity));
  }
  return { lines, synced, offsetMs };
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
