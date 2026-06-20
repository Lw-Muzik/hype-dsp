import { Sparkles } from "lucide-react";
import { useUiStore } from "@/stores/ui";

/**
 * A subtle trial-countdown banner. Upgrades are managed by an administrator, so
 * there's no self-serve activation here — the auth gate (`AuthGate`) blocks
 * access entirely once a trial ends. Licensed users see nothing.
 */
export function TrialBanner() {
  const license = useUiStore((s) => s.license);
  if (!license || license.kind !== "trial") return null;

  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center gap-2 border-b border-border bg-surface-raised px-5 py-2 text-sm text-text-muted"
    >
      <Sparkles className="size-4" aria-hidden="true" />
      Trial — {license.daysLeft} day{license.daysLeft === 1 ? "" : "s"} left.
    </div>
  );
}
