import { useUpdateStore } from "@/stores/update";
import { noticeFor } from "./updateNotice";

/**
 * Top-bar companion to `UpdateBanner`: shows download progress, and after
 * "Later" keeps a ready update visible — clicking it restores the banner.
 * Renders nothing when there is nothing to say.
 */
export function UpdatePill() {
  const status = useUpdateStore((s) => s.status);
  const dismissedVersion = useUpdateStore((s) => s.dismissedVersion);
  const reopen = useUpdateStore((s) => s.reopen);

  const notice = noticeFor(status, dismissedVersion);
  if (notice?.kind !== "pill") return null;

  if (notice.phase === "downloading") {
    const pct = notice.percent;
    return (
      <span
        role="status"
        aria-label={
          pct === null
            ? `Downloading update ${notice.version}`
            : `Downloading update ${notice.version}, ${pct} percent`
        }
        className="inline-flex items-center gap-2 rounded-full border border-border bg-surface-raised px-3 py-1 text-xs text-text-muted"
      >
        <span className="relative h-1 w-8 overflow-hidden rounded-full bg-surface-overlay" aria-hidden="true">
          <span
            className={
              pct === null
                ? "absolute inset-y-0 left-0 w-1/2 rounded-full bg-accent motion-safe:animate-pulse"
                : "absolute inset-y-0 left-0 rounded-full bg-accent transition-[width] duration-300"
            }
            style={pct === null ? undefined : { width: `${pct}%` }}
          />
        </span>
        <span className="tabular-nums">{pct === null ? "Updating…" : `Updating… ${pct}%`}</span>
      </span>
    );
  }

  return (
    <button
      type="button"
      onClick={reopen}
      aria-label={`Update ${notice.version} ready — show details`}
      className="inline-flex items-center gap-2 rounded-full border border-accent/40 bg-accent/15 px-3 py-1 text-xs font-medium text-accent transition-colors hover:bg-accent/25"
    >
      <span className="relative flex size-2" aria-hidden="true">
        <span className="absolute inline-flex size-full rounded-full bg-accent opacity-60 motion-safe:animate-ping" />
        <span className="relative inline-flex size-2 rounded-full bg-accent" />
      </span>
      Update ready
    </button>
  );
}
