import { cn } from "@/lib/cn";

export interface SegmentedItem<T extends string> {
  value: T;
  label: string;
}

interface SegmentedProps<T extends string> {
  items: readonly SegmentedItem<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Names the group for screen readers. */
  label: string;
  className?: string;
}

/** A small exclusive choice, rendered inline rather than behind a dropdown. */
export function Segmented<T extends string>({
  items, value, onChange, label, className,
}: SegmentedProps<T>) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn(
        "flex gap-1 rounded-control border border-border bg-surface-raised p-1",
        className,
      )}
    >
      {items.map((item) => {
        const active = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(item.value)}
            className={cn(
              "flex-1 rounded-[7px] px-3 py-1.5 text-sm font-medium transition-colors",
              active ? "bg-surface-overlay text-text" : "text-text-muted hover:text-text",
            )}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
