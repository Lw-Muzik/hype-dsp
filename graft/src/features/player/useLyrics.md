# src/features/player/useLyrics.ts

- cacheSet · function · L14-L21 — function cacheSet(key: string, value: ParsedLyrics | null): void
- clearLyricsCache · function · L25-L33 — function clearLyricsCache(key?: string): void
- LyricsState · interface · L35-L40 — interface LyricsState
- resolveRaw · function · L46-L62 — async function resolveRaw( item: QueueItem | undefined, title: string, artist: string | null, durationSecs: number | null, ): Promise<string | null>
- fetchFor · function · L64-L83 — function fetchFor( key: string, item: QueueItem | undefined, title: string, artist: string | null, durationSecs: number | null, ): Promise<ParsedLyrics | null>
- useLyrics · function · L90-L128 — function useLyrics(): LyricsState
