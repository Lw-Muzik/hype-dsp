# src/features/player/TrackRow.tsx

- RowSource · type · L10-L10 — type RowSource = "local" | "phone" | "cloud" | "ytmusic";
- TrackRow · function · L33-L123 — function TrackRow({ rank, title, artist, durationSecs, art, seed, source = "local", unavailable = false, playing, onPlay, trailing, }: { rank: number; title: string; artist: string | null; durationSecs: number | null; /** Where to resolve the cover art from (omit for gradient-only). */ art?: ArtSource | null; /** Gradient-fallback seed (usually album or title). */ seed: string; source?: RowSource; /** Listed but not playable (a removed / region-blocked YT Music track). */ unavailable?: boolean; playing: boolean; onPlay: () => void; trailing?: ReactNode; })
