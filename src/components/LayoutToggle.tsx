import { LayoutGrid, List } from "lucide-react";
import { cn } from "@/lib/cn";

export type LayoutMode = "list" | "grid";

/** A compact list/grid view switch, shared by the Stations panels. */
export function LayoutToggle({
  value,
  onChange,
}: {
  value: LayoutMode;
  onChange: (mode: LayoutMode) => void;
}) {
  return (
    <div className="flex items-center gap-1 rounded-control border border-border bg-surface-raised p-1">
      {([["list", List], ["grid", LayoutGrid]] as const).map(([mode, Icon]) => (
        <button
          key={mode}
          type="button"
          onClick={() => onChange(mode)}
          aria-label={mode === "list" ? "List view" : "Grid view"}
          aria-pressed={value === mode}
          className={cn(
            "flex size-8 items-center justify-center rounded-[7px] transition-colors",
            value === mode
              ? "bg-surface-overlay text-text"
              : "text-text-muted hover:text-text",
          )}
        >
          <Icon className="size-4" aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}
