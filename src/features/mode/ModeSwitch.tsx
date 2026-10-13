import { useEffect, useRef, useState } from "react";
import { Loader2, LibraryBig, SlidersVertical } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { systemAudioStatus } from "@/lib/ipc";
import { useSystemEqStore } from "@/stores/systemEq";
import { cn } from "@/lib/cn";
import { modeFromSystemEq, type AppMode } from "./appMode";
import { switchMode } from "./switchMode";

const OPTIONS: { mode: AppMode; label: string; icon: LucideIcon }[] = [
  { mode: "player", label: "Player", icon: LibraryBig },
  { mode: "equalizer", label: "Equalizer", icon: SlidersVertical },
];

const UNSUPPORTED = "Equalizer mode isn't available on this computer";

/**
 * Two-segment "Player | Equalizer" switch. The mode is the persisted system-wide
 * EQ intent, so the thumb can never claim Equalizer while nothing is equalizing.
 */
export function ModeSwitch() {
  const mode = modeFromSystemEq(useSystemEqStore((s) => s.enabled));
  const [pending, setPending] = useState<AppMode | null>(null);
  const [supported, setSupported] = useState(true);
  const refs = useRef<Record<AppMode, HTMLButtonElement | null>>({
    player: null,
    equalizer: null,
  });

  useEffect(() => {
    let cancelled = false;
    systemAudioStatus()
      .then((s) => !cancelled && setSupported(s.supported !== false))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const go = async (target: AppMode) => {
    if (pending || target === mode) return;
    setPending(target);
    try {
      await switchMode(target);
    } finally {
      setPending(null);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent, index: number) => {
    const step =
      e.key === "ArrowRight" || e.key === "ArrowDown"
        ? 1
        : e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? -1
          : 0;
    if (!step) return;
    e.preventDefault();
    const next = OPTIONS[(index + step + OPTIONS.length) % OPTIONS.length]!;
    refs.current[next.mode]?.focus();
  };

  const shown = pending ?? mode;

  return (
    <div
      role="radiogroup"
      aria-label="Mode"
      className="relative flex shrink-0 items-center rounded-control border border-border bg-surface-raised p-0.5"
    >
      {/* Sliding thumb under the active segment. */}
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-[8px] bg-accent shadow-sm transition-transform duration-200 ease-out",
          shown === "equalizer" && "translate-x-full",
        )}
      />
      {OPTIONS.map(({ mode: m, label, icon: Icon }, i) => {
        const checked = mode === m;
        const disabled = m === "equalizer" && !supported;
        const active = shown === m;
        return (
          <button
            key={m}
            ref={(el) => {
              refs.current[m] = el;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            disabled={disabled}
            title={disabled ? UNSUPPORTED : undefined}
            tabIndex={checked ? 0 : -1}
            onClick={() => void go(m)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={cn(
              "relative z-10 flex flex-1 items-center justify-center gap-1.5 rounded-[8px] px-3 py-1.5 text-xs font-medium transition-colors",
              active ? "text-on-accent" : "text-text-muted hover:text-text",
              disabled && "cursor-not-allowed opacity-50 hover:text-text-muted",
            )}
          >
            {pending === m ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <Icon className="size-3.5" aria-hidden="true" />
            )}
            {label}
          </button>
        );
      })}
    </div>
  );
}
