import { useEffect, useRef, useState } from "react";
import { Check, ChevronsUpDown, Search, X } from "lucide-react";
import { cn } from "@/lib/cn";

export interface ComboItem {
  id: string;
  label: string;
  sublabel?: string;
}

interface ComboboxProps {
  items: ComboItem[];
  value: string | null;
  onSelect: (id: string) => void;
  onClear?: () => void;
  placeholder?: string;
  searchPlaceholder?: string;
  emptyText?: string;
}

/** A searchable single-select combobox (filter input + option list). */
export function Combobox({
  items,
  value,
  onSelect,
  onClear,
  placeholder = "Select…",
  searchPlaceholder = "Search…",
  emptyText = "No matches",
}: ComboboxProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  const selected = items.find((i) => i.id === value) ?? null;
  const q = query.trim().toLowerCase();
  const filtered = q
    ? items.filter((i) =>
        `${i.label} ${i.sublabel ?? ""}`.toLowerCase().includes(q),
      )
    : items;

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <div className="flex items-center gap-1">
        <button
          type="button"
          role="combobox"
          aria-expanded={open}
          aria-haspopup="listbox"
          onClick={() => setOpen((o) => !o)}
          className="flex flex-1 items-center justify-between gap-2 rounded-control border border-border bg-surface px-3 py-2 text-left text-sm hover:border-border-strong"
        >
          <span className={cn("truncate", !selected && "text-text-muted")}>
            {selected ? selected.label : placeholder}
          </span>
          <ChevronsUpDown
            className="size-4 shrink-0 text-text-faint"
            aria-hidden="true"
          />
        </button>
        {selected && onClear && (
          <button
            type="button"
            aria-label="Clear selection"
            onClick={() => {
              onClear();
              setQuery("");
            }}
            className="rounded-control border border-border p-2 text-text-faint hover:text-danger"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        )}
      </div>

      {open && (
        <div className="absolute z-20 mt-1 w-full overflow-hidden rounded-control border border-border bg-surface-raised shadow-lg">
          <div className="flex items-center gap-2 border-b border-border px-3">
            <Search className="size-4 text-text-faint" aria-hidden="true" />
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={searchPlaceholder}
              aria-label={searchPlaceholder}
              className="w-full bg-transparent py-2 text-sm outline-none placeholder:text-text-faint"
            />
          </div>
          <ul className="max-h-64 overflow-y-auto py-1" role="listbox">
            {filtered.length === 0 && (
              <li className="px-3 py-2 text-sm text-text-muted">{emptyText}</li>
            )}
            {filtered.map((item) => {
              const active = item.id === value;
              return (
                <li key={item.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={active}
                    onClick={() => {
                      onSelect(item.id);
                      setOpen(false);
                      setQuery("");
                    }}
                    className={cn(
                      "flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-sm hover:bg-surface-overlay",
                      active && "text-accent-strong",
                    )}
                  >
                    <span className="min-w-0">
                      <span className="block truncate">{item.label}</span>
                      {item.sublabel && (
                        <span className="block truncate text-xs text-text-faint">
                          {item.sublabel}
                        </span>
                      )}
                    </span>
                    {active && (
                      <Check className="size-4 shrink-0" aria-hidden="true" />
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
