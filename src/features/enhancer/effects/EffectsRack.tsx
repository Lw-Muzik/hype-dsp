import { useUiStore } from "@/stores/ui";
import { Switch } from "@/components/Switch";
import { cn } from "@/lib/cn";
import { useEffectToggles } from "./useEffectToggles";
import type { EffectToggle } from "./useEffectToggles";
import { EFFECTS } from "./useEffectToggles";

const FLASH_CLASSES = ["ring-2", "ring-accent", "ring-offset-2", "ring-offset-canvas"];
const FLASH_MS = 1000;

function prefersReducedMotion(): boolean {
  try {
    return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  } catch {
    return false;
  }
}

/** Scroll the card for `anchor` into view and flash a ring around it. */
function jumpToCard(anchor: string) {
  const el = document.getElementById(anchor);
  if (!el) return;
  el.scrollIntoView({
    behavior: prefersReducedMotion() ? "auto" : "smooth",
    block: "start",
  });
  el.classList.add("transition-shadow", ...FLASH_CLASSES);
  window.setTimeout(() => el.classList.remove(...FLASH_CLASSES), FLASH_MS);
}

function Tile({ fx }: { fx: EffectToggle }) {
  const Icon = fx.icon;
  const isEq = fx.id === "equalizer";
  const jump = () => {
    if (isEq) useUiStore.getState().setRoute("equalizer");
    else jumpToCard(fx.anchor);
  };
  const lockedOff = !fx.enabled && !fx.canToggleOn;

  return (
    <li
      data-effect={fx.id}
      data-on={fx.enabled}
      className={cn(
        "flex items-center gap-1 rounded-control border pr-2 transition-colors",
        fx.enabled
          ? "border-accent/40 bg-accent-muted"
          : "border-border bg-surface-raised hover:border-border-strong",
      )}
    >
      <button
        type="button"
        onClick={jump}
        title={isEq ? "Open the Equalizer" : `Go to ${fx.label}`}
        className={cn(
          "flex min-w-0 flex-1 items-center gap-2 rounded-control px-2.5 py-2 text-left text-sm",
          "focus-visible:outline-2 focus-visible:outline-accent",
          fx.enabled ? "text-accent-strong" : "text-text-muted hover:text-text",
        )}
      >
        <Icon className="size-4 shrink-0" aria-hidden="true" />
        <span className="truncate font-medium">{fx.label}</span>
      </button>
      {fx.hasSwitch && (
        <span
          title={lockedOff ? "Choose a headphone profile to turn this on" : undefined}
          className={cn("shrink-0", lockedOff && "opacity-40")}
        >
          <Switch
            checked={fx.enabled}
            onChange={fx.toggle}
            label={fx.switchLabel}
            disabled={lockedOff}
          />
        </span>
      )}
    </li>
  );
}

/**
 * Compact, sticky overview of every effect: see what's on, toggle it, or jump
 * to its card. Shares its toggles with the cards via `useEffectToggles`.
 */
export function EffectsRack() {
  const toggles = useEffectToggles();
  const onCount = EFFECTS.filter((e) => toggles[e.id].hasSwitch && toggles[e.id].enabled).length;

  return (
    <nav
      aria-label="Effects rack"
      className="sticky top-0 z-10 -mx-2 rounded-card border border-border bg-canvas/80 px-2 py-2 shadow-sm backdrop-blur-md supports-[backdrop-filter]:bg-canvas/60"
    >
      <p className="mb-2 px-1 text-xs font-medium uppercase tracking-wide text-text-muted">
        Effects · <span className="text-accent-strong">{onCount} on</span>
      </p>
      <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
        {EFFECTS.map((e) => (
          <Tile key={e.id} fx={toggles[e.id]} />
        ))}
      </ul>
    </nav>
  );
}
