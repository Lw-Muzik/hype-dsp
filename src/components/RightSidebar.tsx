import { useEffect, useState } from "react";
import { ListMusic, MicVocal, Video, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import { QueueList } from "@/features/player/QueueList";
import { LyricsView } from "@/features/player/LyricsView";
import { VideoStage } from "@/features/player/VideoStage";
import { cn } from "@/lib/cn";

const ANIM_MS = 220;

/**
 * The show/hide right sidebar that hosts the play Queue, Lyrics and Video,
 * toggled from the now-playing bar. It lives in the layout (not an overlay), so
 * it animates its width open/closed and the content beside it reflows smoothly.
 * The panel content stays mounted through the close animation, then unmounts —
 * so it slides out rather than snapping (reduced-motion collapses the timing).
 *
 * Video is offered only for a track that has footage, and is a picture only:
 * the sound stays in the engine's enhancement chain throughout. See
 * [`VideoStage`].
 */
export function RightSidebar() {
  const panel = useUiStore((s) => s.rightPanel);
  const toggleRight = useUiStore((s) => s.toggleRight);
  const closeRight = useUiStore((s) => s.closeRight);
  const rightWidth = useUiStore((s) => s.rightWidth);
  const resizing = useUiStore((s) => s.resizing);

  // The tab whose content is mounted. Lingers briefly after close so the
  // panel can animate out before its content disappears.
  const [rendered, setRendered] = useState(panel);
  useEffect(() => {
    if (panel) {
      setRendered(panel);
      return;
    }
    const t = window.setTimeout(() => setRendered(null), ANIM_MS);
    return () => window.clearTimeout(t);
  }, [panel]);

  // Only a music video has anything to watch: a song is an audio entity YouTube
  // renders as a square still, so a Video tab there would promise a picture and
  // hand back the cover art.
  const current = useEngineStore((s) =>
    s.queueIndex >= 0 ? s.queue[s.queueIndex] : undefined,
  );
  const videoId = current?.ytTrack?.hasVideo ? current.ytTrack.videoId : null;

  // Losing the tab under you (skipping to a song) shouldn't leave the panel on a
  // tab that no longer exists.
  useEffect(() => {
    if (!videoId && panel === "video") toggleRight("video");
  }, [videoId, panel, toggleRight]);

  const open = panel !== null;
  const tab = panel ?? rendered;

  return (
    <div
      aria-hidden={!open}
      style={{ width: open ? rightWidth : 0 }}
      className={cn(
        "h-full shrink-0 overflow-hidden ease-out",
        // Drop the transition mid-drag so the width tracks the cursor exactly.
        !resizing && "transition-[width] duration-200",
      )}
    >
      <aside
        aria-label={tab === "lyrics" ? "Lyrics" : tab === "video" ? "Video" : "Play queue"}
        style={{ width: rightWidth }}
        className={cn(
          "flex h-full flex-col bg-surface-raised transition-opacity duration-200",
          open ? "opacity-100" : "opacity-0",
        )}
      >
        <div className="flex h-12 shrink-0 items-center justify-between px-2 border-b border-border">
          <div className="flex items-center gap-1" role="tablist" aria-label="Right panel">
            <Tab
              icon={ListMusic}
              label="Queue"
              active={tab === "queue"}
              onClick={() => tab !== "queue" && toggleRight("queue")}
            />
            <Tab
              icon={MicVocal}
              label="Lyrics"
              active={tab === "lyrics"}
              onClick={() => tab !== "lyrics" && toggleRight("lyrics")}
            />
            {videoId && (
              <Tab
                icon={Video}
                label="Video"
                active={tab === "video"}
                onClick={() => tab !== "video" && toggleRight("video")}
              />
            )}
          </div>
          <button
            type="button"
            aria-label="Close panel"
            onClick={closeRight}
            className="grid size-7 place-items-center rounded-control text-text-faint transition-colors hover:bg-surface-overlay hover:text-text"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        </div>

        {rendered === "queue" ? (
          <QueueList />
        ) : rendered === "lyrics" ? (
          <LyricsView />
        ) : rendered === "video" && videoId ? (
          // Keyed by track: a new video is a new element, not a reused one that
          // has to be talked out of the previous track's buffer and position.
          <div className="p-3">
            <VideoStage key={videoId} videoId={videoId} />
          </div>
        ) : null}
      </aside>
    </div>
  );
}

function Tab({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-control px-3 py-1.5 text-sm font-medium transition-colors",
        active ? "bg-surface-overlay text-text" : "text-text-muted hover:text-text",
      )}
    >
      <Icon className="size-4" aria-hidden="true" />
      {label}
    </button>
  );
}
