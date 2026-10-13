# src/lib/lrc.ts

- LyricWord · interface · L8-L12 — interface LyricWord
- LyricLine · interface · L14-L23 — interface LyricLine
- ParsedLyrics · interface · L25-L32 — interface ParsedLyrics
- toMs · function · L40-L44 — function toMs(min: string, sec: string, frac: string | undefined): number
- parseWords · function · L47-L61 — function parseWords(body: string): LyricWord[] | undefined
- parseLrc · function · L63-L111 — function parseLrc(raw: string): ParsedLyrics
- activeLineIndex · function · L114-L132 — function activeLineIndex( lines: LyricLine[], positionMs: number, ): number
