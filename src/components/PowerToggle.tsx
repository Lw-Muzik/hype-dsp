import { Power } from "lucide-react";
import { cn } from "@/lib/cn";

interface PowerToggleProps {
  on: boolean;
  onToggle: (on: boolean) => void;
}

/**
 * The global enhancement power switch. A proper ARIA switch: it announces its
 * on/off state and toggles on click, Enter, or Space.
 */
export function PowerToggle({ on, onToggle }: PowerToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={on ? "Turn enhancement off" : "Turn enhancement on"}
      onClick={() => onToggle(!on)}
      className={cn(
        "inline-flex items-center gap-2 rounded-control border px-3 py-1.5 text-sm font-medium transition-colors",
        on
          ? "border-accent/40 bg-accent-muted text-accent-strong"
          : "border-border bg-surface-raised text-text-muted hover:text-text",
      )}
    >
      <Power className="size-4" aria-hidden="true" />
      <span>{on ? "On" : "Off"}</span>
    </button>
  );
}
