import { cn } from "@/lib/cn";

/**
 * A horizontal row of filter pills (the library's genres, with "All" first).
 * The active pill is filled with the accent; the rest are quiet outlines.
 */
export function CategoryChips({
  categories,
  active,
  onSelect,
}: {
  categories: string[];
  active: string;
  onSelect: (category: string) => void;
}) {
  if (categories.length <= 1) return null;
  return (
    <div
      className="flex gap-2 overflow-x-auto pb-1"
      role="tablist"
      aria-label="Genres"
    >
      {categories.map((c) => {
        const selected = c === active;
        return (
          <button
            key={c}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => onSelect(c)}
            className={cn(
              "shrink-0 rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors",
              selected
                ? "bg-accent text-on-accent"
                : "border border-border text-text-muted hover:border-border-strong hover:text-text",
            )}
          >
            {c}
          </button>
        );
      })}
    </div>
  );
}
