import { useState } from "react";
import { Sparkles } from "lucide-react";
import { Button } from "@/components/Button";
import { useUpdateStore } from "@/stores/update";
import { noticeFor } from "./updateNotice";

/**
 * Full-width notice under the top bar once an update is downloaded and staged.
 * "Later" only collapses it into the top-bar pill (`UpdatePill`); the update is
 * never hidden for good. Notes render as plain text — they come from a release
 * manifest, so no HTML or markdown is interpreted.
 */
export function UpdateBanner() {
  const status = useUpdateStore((s) => s.status);
  const dismissedVersion = useUpdateStore((s) => s.dismissedVersion);
  const restarting = useUpdateStore((s) => s.restarting);
  const dismiss = useUpdateStore((s) => s.dismiss);
  const restart = useUpdateStore((s) => s.restart);
  const [showNotes, setShowNotes] = useState(false);

  const notice = noticeFor(status, dismissedVersion);
  if (notice?.kind !== "banner") return null;

  return (
    <div
      role="status"
      aria-live="polite"
      className="shrink-0 border-b border-accent/30 bg-accent/15"
    >
      <div className="flex min-h-11 items-center gap-3 px-5 py-1.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent/20 text-accent">
          <Sparkles className="size-4" aria-hidden="true" />
        </span>
        <p className="min-w-0 flex-1 truncate text-sm">
          <span className="font-medium">HypeMuzik {notice.version} is ready</span>
          <span className="ml-2 hidden text-text-muted sm:inline">
            Restart to update — it takes a few seconds.
          </span>
        </p>
        <div className="flex shrink-0 items-center gap-1.5">
          {notice.notes && (
            <Button
              variant="ghost"
              className="px-2.5 py-1 text-xs"
              aria-expanded={showNotes}
              onClick={() => setShowNotes((v) => !v)}
            >
              What&apos;s new
            </Button>
          )}
          <Button
            variant="ghost"
            className="px-2.5 py-1 text-xs text-text-faint"
            onClick={dismiss}
          >
            Later
          </Button>
          <Button
            variant="primary"
            className="px-3 py-1 text-xs"
            disabled={restarting}
            onClick={() => void restart()}
          >
            {restarting ? "Restarting…" : "Restart now"}
          </Button>
        </div>
      </div>
      {showNotes && notice.notes && (
        <p className="max-h-40 overflow-y-auto whitespace-pre-wrap border-t border-accent/20 px-5 py-3 pl-[3.25rem] text-xs leading-relaxed text-text-muted">
          {notice.notes}
        </p>
      )}
    </div>
  );
}
