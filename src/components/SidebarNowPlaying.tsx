import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import { useRemoteStore } from "@/features/connect/remote/store";
import { selectNowPlaying } from "@/features/connect/remote/nowPlaying";
import { useRemotePosition } from "@/features/connect/remote/ui/useRemotePosition";
import type { RemoteView } from "@/features/connect/remote/session";
import { coverGradient, coverInitials } from "@/lib/cover";

/**
 * A large cover of the currently-playing track, docked near the bottom of the
 * sidebar. Uses the now-playing card's already-decoded art (gradient + initials
 * fallback), collapses to a thumbnail with the rail, and opens the Player when
 * clicked. Renders nothing when nothing is playing.
 */
export function SidebarNowPlaying({ collapsed }: { collapsed: boolean }) {
  const localMeta = useEngineStore((s) => s.nowPlayingMeta);
  // While another device plays (Hype Connect remote mode) show its track.
  const remote = useRemoteStore((s) => s.view);
  const { meta } = selectNowPlaying(localMeta, remote);
  const setRoute = useUiStore((s) => s.setRoute);

  if (!meta) return null;

  const title = meta.title ?? "Unknown";
  const subtitle = meta.artist ?? meta.album ?? null;
  const seed = meta.album?.trim() || title;

  const cover = meta.cover ? (
    <img
      src={meta.cover}
      alt=""
      aria-hidden="true"
      className="size-full object-cover"
    />
  ) : (
    <div
      aria-hidden="true"
      className="grid size-full place-items-center text-2xl font-semibold text-white/90"
      style={{ background: coverGradient(seed) }}
    >
      <span className="opacity-80">{coverInitials(title)}</span>
    </div>
  );

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={() => setRoute("player")}
        aria-label={`Now playing: ${title}. Open player`}
        title={subtitle ? `${title} — ${subtitle}` : title}
        className="mx-auto mb-2 block size-11 overflow-hidden rounded-lg shadow-md ring-1 ring-white/10 transition-transform hover:scale-105"
      >
        {cover}
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={() => setRoute("player")}
      aria-label={`Now playing: ${title}. Open player`}
      className="group mx-3 mb-2 block text-left"
    >
      <div className="relative aspect-square w-full overflow-hidden rounded-card shadow-lg ring-1 ring-white/10">
        {cover}
        <span className="pointer-events-none absolute left-2.5 top-2.5 rounded-full bg-black/55 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-white/90 backdrop-blur">
          {remote ? `On ${remote.device.name}` : "Now playing"}
        </span>
      </div>
      <p className="mt-2 truncate font-display text-sm font-medium">{title}</p>
      {subtitle && (
        <p className="truncate text-xs text-text-muted">{subtitle}</p>
      )}
      {remote ? <RemoteProgress view={remote} /> : <LocalProgress />}
    </button>
  );
}

/** Thin progress line under the card; the bar itself is the full control. */
function ProgressLine({ position, duration }: { position: number; duration: number }) {
  if (!(duration > 0)) return null;
  const pct = Math.min(100, Math.max(0, (position / duration) * 100));
  return (
    <div
      role="progressbar"
      aria-label="Playback progress"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(position)}
      className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-white/10"
    >
      <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
    </div>
  );
}

function LocalProgress() {
  const position = useEngineStore((s) => s.positionSecs);
  const duration = useEngineStore((s) => s.durationSecs);
  return <ProgressLine position={position} duration={duration ?? 0} />;
}

function RemoteProgress({ view }: { view: RemoteView }) {
  const ms = useRemotePosition(view);
  const durationMs = view.durationMs ?? view.track?.durationMs ?? 0;
  return <ProgressLine position={ms / 1000} duration={durationMs / 1000} />;
}
