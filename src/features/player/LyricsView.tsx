import { SyncedLyrics } from "@/features/player/SyncedLyrics";
import { RemoteGate } from "@/features/connect/remote/ui/RemoteNotice";

/**
 * The Lyrics tab — full-height synced lyrics for the current track.
 *
 * A thin wrapper over {@link SyncedLyrics}, which is the shared renderer used
 * both here and under the video player. Kept as its own name so the tab reads as
 * a tab, and so any future tab-only chrome has a home.
 */
export function LyricsView() {
  // Another device's lyrics are not streamed here: say where they are instead
  // of showing the (stale) local ones.
  return (
    <RemoteGate what="Lyrics">
      <SyncedLyrics />
    </RemoteGate>
  );
}
