import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
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

/** Where the floating panel sits relative to the trigger. */
interface Placement {
  left: number;
  width: number;
  /** Distance from the top of the viewport (when opening downward). */
  top?: number;
  /** Distance from the bottom of the viewport (when flipped upward). */
  bottom?: number;
  /** Largest the panel may grow before it scrolls internally. */
  maxHeight: number;
  openUp: boolean;
}

const GAP = 6; // px between the trigger and the panel
const MIN_PANEL = 180; // px of room needed below before we flip up
const MAX_PANEL = 340; // px the panel is allowed to grow to

/**
 * A searchable single-select combobox: a filter input over a scrollable option
 * list, with full keyboard control (↑/↓/Home/End to move, Enter to pick, Esc to
 * close). The panel is portalled and anchored to the trigger so it floats above
 * surrounding content, flips up when there's no room below, and never clips.
 */
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
  const [active, setActive] = useState(0);
  const [place, setPlace] = useState<Placement | null>(null);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const baseId = useId();

  const selected = items.find((i) => i.id === value) ?? null;
  const q = query.trim().toLowerCase();
  const filtered = q
    ? items.filter((i) =>
        `${i.label} ${i.sublabel ?? ""}`.toLowerCase().includes(q),
      )
    : items;

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
  }, []);

  const pick = useCallback(
    (id: string) => {
      onSelect(id);
      close();
      triggerRef.current?.focus();
    },
    [onSelect, close],
  );

  // Position the panel against the trigger; flip up when below is too tight.
  const reposition = useCallback(() => {
    const t = triggerRef.current;
    if (!t) return;
    const r = t.getBoundingClientRect();
    // Close if the trigger has scrolled out of view entirely.
    if (r.bottom < 0 || r.top > window.innerHeight) {
      setOpen(false);
      return;
    }
    const below = window.innerHeight - r.bottom - GAP;
    const above = r.top - GAP;
    const openUp = below < MIN_PANEL && above > below;
    const maxHeight = Math.min(MAX_PANEL, Math.max(120, openUp ? above : below));
    setPlace({
      left: r.left,
      width: r.width,
      maxHeight,
      openUp,
      ...(openUp
        ? { bottom: window.innerHeight - r.top + GAP }
        : { top: r.bottom + GAP }),
    });
  }, []);

  // On open: reset the query, pre-highlight the current selection, and place
  // the panel synchronously so it never paints in the wrong spot. Keyed on the
  // open transition only — items/value are read fresh and must not re-trigger
  // this (that would wipe an in-progress search).
  useLayoutEffect(() => {
    if (!open) return;
    setQuery("");
    const sel = items.findIndex((i) => i.id === value);
    setActive(sel >= 0 ? sel : 0);
    reposition();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Keep the panel anchored while the user scrolls or resizes the window.
  useEffect(() => {
    if (!open) return;
    const onScroll = () => reposition();
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open, reposition]);

  // Dismiss on an outside click (the panel lives in a portal, so check both).
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (panelRef.current?.contains(target)) return;
      close();
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open, close]);

  // Clamp the highlight when the filtered set shrinks, and scroll it into view.
  useEffect(() => {
    if (active > filtered.length - 1) setActive(Math.max(0, filtered.length - 1));
  }, [filtered.length, active]);

  useEffect(() => {
    if (!open) return;
    listRef.current
      ?.querySelector(`[data-idx="${active}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const onTriggerKey = (e: ReactKeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setOpen(true);
    }
  };

  const onListKey = (e: ReactKeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      triggerRef.current?.focus();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => (filtered.length ? (a + 1) % filtered.length : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (filtered.length ? (a - 1 + filtered.length) % filtered.length : 0));
    } else if (e.key === "Home") {
      e.preventDefault();
      setActive(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setActive(Math.max(0, filtered.length - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[active];
      if (item) pick(item.id);
    }
  };

  return (
    <div className="relative">
      <div className="flex items-center gap-1.5">
        <button
          ref={triggerRef}
          type="button"
          role="combobox"
          aria-expanded={open}
          aria-haspopup="listbox"
          aria-controls={open ? `${baseId}-list` : undefined}
          onClick={() => setOpen((o) => !o)}
          onKeyDown={onTriggerKey}
          className={cn(
            "flex flex-1 items-center justify-between gap-2 rounded-control border bg-surface px-3 py-2.5 text-left text-sm transition-colors",
            open ? "border-accent" : "border-border hover:border-border-strong",
          )}
        >
          <span className={cn("truncate", !selected && "text-text-muted")}>
            {selected ? selected.label : placeholder}
          </span>
          <ChevronsUpDown
            className={cn(
              "size-4 shrink-0 transition-colors",
              open ? "text-accent" : "text-text-faint",
            )}
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
            className="rounded-control border border-border p-2.5 text-text-faint transition-colors hover:border-border-strong hover:text-danger"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        )}
      </div>

      {open &&
        place &&
        createPortal(
          <div
            ref={panelRef}
            className="hm-pop fixed z-50 overflow-hidden rounded-control border border-border-strong bg-surface-overlay shadow-2xl ring-1 ring-black/40"
            style={{
              left: place.left,
              width: place.width,
              top: place.top,
              bottom: place.bottom,
              transformOrigin: place.openUp ? "bottom" : "top",
            }}
            onKeyDown={onListKey}
          >
            <div className="flex items-center gap-2 border-b border-border px-3">
              <Search className="size-4 shrink-0 text-text-faint" aria-hidden="true" />
              <input
                ref={inputRef}
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={searchPlaceholder}
                aria-label={searchPlaceholder}
                aria-controls={`${baseId}-list`}
                aria-activedescendant={
                  filtered[active] ? `${baseId}-opt-${active}` : undefined
                }
                className="w-full bg-transparent py-2.5 text-sm outline-none placeholder:text-text-faint"
              />
              {query && (
                <button
                  type="button"
                  aria-label="Clear search"
                  onClick={() => {
                    setQuery("");
                    inputRef.current?.focus();
                  }}
                  className="shrink-0 rounded p-0.5 text-text-faint transition-colors hover:text-text"
                >
                  <X className="size-3.5" aria-hidden="true" />
                </button>
              )}
            </div>
            <ul
              ref={listRef}
              id={`${baseId}-list`}
              role="listbox"
              className="overflow-y-auto py-1"
              style={{ maxHeight: place.maxHeight - 46 }}
            >
              {filtered.length === 0 ? (
                <li className="px-3 py-6 text-center text-sm text-text-muted">
                  {emptyText}
                </li>
              ) : (
                filtered.map((item, idx) => {
                  const isSelected = item.id === value;
                  const isActive = idx === active;
                  return (
                    <li key={item.id}>
                      <button
                        type="button"
                        role="option"
                        id={`${baseId}-opt-${idx}`}
                        data-idx={idx}
                        aria-selected={isSelected}
                        onMouseEnter={() => setActive(idx)}
                        onClick={() => pick(item.id)}
                        className={cn(
                          "relative flex w-full items-center justify-between gap-2 px-3 py-2.5 text-left text-sm transition-colors",
                          isActive ? "bg-surface-overlay" : "bg-transparent",
                        )}
                      >
                        {isActive && (
                          <span
                            className="absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-accent"
                            aria-hidden="true"
                          />
                        )}
                        <span className="min-w-0">
                          <span
                            className={cn(
                              "block truncate",
                              isSelected && "font-medium text-accent-strong",
                            )}
                          >
                            {item.label}
                          </span>
                          {item.sublabel && (
                            <span className="block truncate text-xs text-text-faint">
                              {item.sublabel}
                            </span>
                          )}
                        </span>
                        {isSelected && (
                          <Check
                            className="size-4 shrink-0 text-accent-strong"
                            aria-hidden="true"
                          />
                        )}
                      </button>
                    </li>
                  );
                })
              )}
            </ul>
            {filtered.length > 0 && (
              <div className="border-t border-border px-3 py-1.5 text-right text-xs text-text-faint">
                {filtered.length}
                {q ? ` of ${items.length}` : ""}{" "}
                {filtered.length === 1 ? "option" : "options"}
              </div>
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}
