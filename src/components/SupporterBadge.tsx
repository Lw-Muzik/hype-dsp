import { Sparkles } from "lucide-react";
import { cn } from "@/lib/cn";

/**
 * The gold "Supporter" badge. Uses the accent tokens (gold in dark, a deeper
 * gold in light) so it stays legible in every theme.
 */
export function SupporterBadge({
  className,
  compact = false,
}: {
  className?: string;
  /** Icon-only dot form for the collapsed sidebar rail. */
  compact?: boolean;
}) {
  if (compact) {
    return (
      <span
        title="Supporter"
        role="img"
        aria-label="Supporter"
        data-testid="supporter-badge"
        className={cn(
          "grid size-4 place-items-center rounded-full border border-accent/40 bg-accent-muted text-accent-strong",
          className,
        )}
      >
        <Sparkles className="size-2.5" aria-hidden="true" />
      </span>
    );
  }
  return (
    <span
      data-testid="supporter-badge"
      className={cn(
        "inline-flex items-center gap-1 rounded-full border border-accent/40 bg-accent-muted px-2 py-0.5 text-[11px] font-semibold leading-none tracking-wide text-accent-strong",
        className,
      )}
    >
      <Sparkles className="size-3" aria-hidden="true" />
      Supporter
    </span>
  );
}
