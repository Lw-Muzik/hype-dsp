# src/features/stations/tvList.ts

- Health · type · L13-L13 — type Health = "alive" | "dead" | "unknown";
- channelHealth · function · L15-L22 — function channelHealth( id: string, probed: ReadonlySet<string>, alive: ReadonlySet<string>, ): Health
- filterChannels · function · L29-L37 — function filterChannels(channels: TvChannel[], query: string): TvChannel[]
- rankByHealth · function · L46-L59 — function rankByHealth( channels: TvChannel[], probed: ReadonlySet<string>, alive: ReadonlySet<string>, ): TvChannel[]
