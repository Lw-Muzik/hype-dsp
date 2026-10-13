import { useRef } from "react";
import { Cast, Play } from "lucide-react";
import { VirtualList } from "@/components/VirtualList";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";
import type { QueueEntry, TrackRef } from "../../protocol";
import { sendRemote } from "../control";
import type { RemoteView } from "../session";

const ROW_H = 68;

function Thumb({ track }: { track: TrackRef }) {
  const seed = track.album?.trim() || track.title;
  return track.artUrl ? (
    <img src={track.artUrl} alt="" aria-hidden="true" className="size-12 rounded-md object-cover" />
  ) : (
    <div
      aria-hidden="true"
      className="grid size-12 place-items-center rounded-md text-base font-semibold text-white/90"
      style={{ background: coverGradient(seed) }}
    >
      <span className="opacity-80">{coverInitials(seed)}</span>
    </div>
  );
}

function Row({ track, current = false, onPick }: { track: TrackRef; current?: boolean; onPick?: () => void }) {
  return (
    <div
      onClick={onPick}
      className={cn(
        "group flex h-full items-center gap-3 rounded-lg px-2 transition-colors",
        onPick && "cursor-pointer hover:bg-surface-overlay",
      )}
    >
      <div className="relative size-12 shrink-0">
        <Thumb track={track} />
        {onPick && (
          <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
            <Play className="size-5 fill-white text-white" aria-hidden="true" />
          </span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p
          className={cn(
            "truncate text-[15px] font-semibold leading-tight",
            current ? "text-accent-strong" : "text-text",
          )}
        >
          {track.title}
        </p>
        <p className="mt-1 truncate text-[13px] text-text-muted">{track.artist ?? "Unknown artist"}</p>
      </div>
    </div>
  );
}

/**
 * The queue while another device plays: its now-playing track and its
 * up-next list (as published in its state). Tapping a row sends `jump`.
 */
export function RemoteQueue({ view }: { view: RemoteView }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const { track, queue, device } = view;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <p
        className={cn(
          "mx-3 mt-3 flex items-center gap-1.5 text-xs font-medium",
          view.playing ? "text-success" : "text-text-faint",
        )}
      >
        <Cast className="size-3.5" aria-hidden="true" />
        {view.playing ? "Playing on" : "Paused on"} {device.name}
      </p>
      {track === null ? (
        <p className="p-4 text-center text-sm text-text-muted">Nothing playing on {device.name}.</p>
      ) : (
        <div className="shrink-0 px-2 pt-3">
          <h3 className="mb-2 px-1 text-base font-semibold tracking-tight">Now playing</h3>
          <div style={{ height: ROW_H }}>
            <Row track={track} current />
          </div>
        </div>
      )}
      <div className="shrink-0 px-3 pb-2 pt-4">
        <h3 className="text-base font-semibold tracking-tight">Next up</h3>
      </div>
      {queue.length === 0 ? (
        <p className="px-3 text-sm text-text-muted">Nothing up next.</p>
      ) : (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          <VirtualList<QueueEntry>
            items={queue}
            rowHeight={ROW_H}
            scrollRef={scrollRef}
            ariaLabel={`Up next on ${device.name}`}
            getKey={(e) => e.uid}
            renderRow={(e) => (
              <Row track={e.track} onPick={() => void sendRemote({ type: "jump", uid: e.uid })} />
            )}
          />
        </div>
      )}
    </div>
  );
}
