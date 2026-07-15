import { useState } from "react";
import { Download, Laptop, Loader2, Smartphone } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Dialog } from "@/components/Dialog";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import { useYtDownloadStore } from "@/stores/ytDownloads";
import { linkPaired } from "@/lib/ipc";
import { formatBytes } from "@/lib/format";
import { THIS_COMPUTER } from "@/lib/platform";
import type { PhoneDevice, YtTrack } from "@/lib/types";
import { cn } from "@/lib/cn";

/** Progress as a percentage, or null while the total is still unknown. */
function percent(bytes: number, total: number | null): number | null {
  if (total == null || total <= 0) return null;
  return Math.min(100, Math.round((bytes / total) * 100));
}

/**
 * A YouTube Music track's download action, for a track row's trailing slot.
 *
 * With no phone paired there's only one place it can go, so the button just
 * downloads — the destination dialog would be a pointless extra click. Downloads
 * need yt-dlp; without it the button stays visible but disabled and says why,
 * because that's a setup step the user can fix, not a missing feature.
 */
export function DownloadAction({ track }: { track: YtTrack }) {
  const ytdlp = useMusicLibraryStore((s) => s.ytdlp);
  const active = useYtDownloadStore((s) => s.active[track.videoId]);
  const toThisComputer = useYtDownloadStore((s) => s.toThisComputer);
  const toPhone = useYtDownloadStore((s) => s.toPhone);
  // Destinations are resolved on click, not on mount: the library is virtualized,
  // so dozens of these mount and unmount as you scroll and a per-row `link_paired`
  // would be an IPC storm. Reading at click time is also fresher.
  const [choosing, setChoosing] = useState<PhoneDevice[] | null>(null);

  // A removed / region-blocked track can't be fetched at all.
  if (!track.isAvailable) return null;

  const ready = ytdlp?.present === true;

  const start = async () => {
    const phones = await linkPaired().catch(() => [] as PhoneDevice[]);
    if (phones.length === 0) {
      void toThisComputer(track.videoId, track.title);
      return;
    }
    setChoosing(phones);
  };

  if (active) {
    const pct = percent(active.bytes, active.total);
    return (
      <span
        className="flex items-center gap-1.5 px-1 text-xs tabular-nums text-text-muted"
        title={
          active.phase === "sending"
            ? "Sending to your phone…"
            : `Downloading… ${formatBytes(active.bytes)}`
        }
      >
        <Loader2 className="size-3.5 animate-spin text-accent" aria-hidden="true" />
        {pct != null ? `${pct}%` : active.phase === "sending" ? "Sending" : "…"}
      </span>
    );
  }

  return (
    <>
      <button
        type="button"
        disabled={!ready}
        aria-label={`Download ${track.title}`}
        title={
          ready
            ? "Download"
            : "Install yt-dlp to download — see YouTube Music in Settings"
        }
        onClick={() => void start()}
        className={cn(
          "grid size-7 place-items-center rounded-control text-text-faint transition-colors",
          ready
            ? "hover:bg-surface hover:text-accent-strong"
            : "cursor-not-allowed opacity-40",
        )}
      >
        <Download className="size-4" aria-hidden="true" />
      </button>

      <Dialog
        open={choosing !== null}
        onClose={() => setChoosing(null)}
        title={`Download “${track.title}”`}
      >
        <div className="flex flex-col gap-2">
          <p className="text-sm text-text-muted">
            Where should it go? It&rsquo;s added to your library either way, so it
            plays offline.
          </p>
          <Destination
            icon={Laptop}
            label={`Download to ${THIS_COMPUTER}`}
            sublabel="Saves to your downloads folder"
            onClick={() => {
              setChoosing(null);
              void toThisComputer(track.videoId, track.title);
            }}
          />
          {choosing?.map((p) => (
            <Destination
              key={p.id}
              icon={Smartphone}
              label={`Download to ${p.name}`}
              sublabel="Downloads here, then sends it over"
              onClick={() => {
                setChoosing(null);
                void toPhone(track.videoId, track.title, p);
              }}
            />
          ))}
        </div>
      </Dialog>
    </>
  );
}

function Destination({
  icon: Icon,
  label,
  sublabel,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  sublabel: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-control border border-border bg-surface px-3 py-2.5 text-left transition-colors hover:border-border-strong hover:bg-surface-overlay"
    >
      <Icon className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
      <span className="min-w-0">
        <span className="block truncate text-sm font-medium">{label}</span>
        <span className="block truncate text-xs text-text-faint">{sublabel}</span>
      </span>
    </button>
  );
}
