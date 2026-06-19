import { useEffect, useState } from "react";
import { ListMusic, MicVocal, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useUiStore } from "@/stores/ui";
import { QueueList } from "@/features/player/QueueList";
import { LyricsView } from "@/features/player/LyricsView";
import { cn } from "@/lib/cn";

const ANIM_MS = 220;

/**
 * The show/hide right sidebar that hosts the play Queue and Lyrics, toggled
 * from the now-playing bar. It lives in the layout (not an overlay), so it
 * animates its width open/closed and the content beside it reflows smoothly.
 * The panel content stays mounted through the close animation, then unmounts —
 * so it slides out rather than snapping (reduced-motion collapses the timing).
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
        aria-label={tab === "lyrics" ? "Lyrics" : "Play queue"}
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

        {rendered === "queue" ? <QueueList /> : rendered === "lyrics" ? <LyricsView /> : null}
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
