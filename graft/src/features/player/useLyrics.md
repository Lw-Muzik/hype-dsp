# src/features/player/useLyrics.ts

- cacheSet · function · L14-L21 — function cacheSet(key: string, value: ParsedLyrics | null): void
- clearLyricsCache · function · L25-L33 — function clearLyricsCache(key?: string): void
- lookupLyrics · function · L38-L47 — function lookupLyrics( key: string, item: QueueItem | undefined, title: string, artist: string | null, durationSecs: number | null, ): Promise<ParsedLyrics | null>
- LyricsState · interface · L49-L54 — interface LyricsState
- resolveRaw · function · L60-L76 — async function resolveRaw( item: QueueItem | undefined, title: string, artist: string | null, durationSecs: number | null, ): Promise<string | null>
- fetchFor · function · L78-L97 — function fetchFor( key: string, item: QueueItem | undefined, title: string, artist: string | null, durationSecs: number | null, ): Promise<ParsedLyrics | null>
- useLyrics · function · L104-L142 — function useLyrics(): LyricsState
