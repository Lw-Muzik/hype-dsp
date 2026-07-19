import { SyncedLyrics } from "@/features/player/SyncedLyrics";

/**
 * The Lyrics tab — full-height synced lyrics for the current track.
 *
 * A thin wrapper over {@link SyncedLyrics}, which is the shared renderer used
 * both here and under the video player. Kept as its own name so the tab reads as
 * a tab, and so any future tab-only chrome has a home.
 */
export function LyricsView() {
  return <SyncedLyrics />;
}
