import type { ReactNode } from "react";
import { Cast } from "lucide-react";
import { Button } from "@/components/Button";
import { playOnDevice } from "../control";
import { useRemoteStore } from "../store";

/**
 * "Lyrics are on <device>": shown instead of stale local data while this
 * computer is only a remote. The other device's lyrics and visualiser are not
 * streamed here (yet), so say where they are and offer the way to get them.
 */
export function RemoteNotice({ what }: { what: string }) {
  const view = useRemoteStore((s) => s.view);
  const selfId = useRemoteStore((s) => s.selfId);
  if (!view) return null;
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <span className="grid size-12 place-items-center rounded-full bg-success/15 text-success">
        <Cast className="size-6" aria-hidden="true" />
      </span>
      <div>
        <p className="font-display text-base font-medium">
          {what} {what.endsWith("s") ? "are" : "is"} on {view.device.name}
        </p>
        <p className="mt-1 max-w-xs text-sm text-text-muted">
          This computer is controlling {view.device.name}. Play here to see {what.toLowerCase()} on this screen.
        </p>
      </div>
      <Button variant="secondary" onClick={() => void playOnDevice(selfId)}>
        Play on this computer
      </Button>
    </div>
  );
}

/** Children in local mode; the notice while this computer is a remote. */
export function RemoteGate({ what, children }: { what: string; children: ReactNode }) {
  const remote = useRemoteStore((s) => s.view !== null);
  return remote ? <RemoteNotice what={what} /> : <>{children}</>;
}
