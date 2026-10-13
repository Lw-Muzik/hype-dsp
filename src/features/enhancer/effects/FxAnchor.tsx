import type { ReactNode } from "react";
import { cn } from "@/lib/cn";
import type { EffectId } from "./useEffectToggles";

/**
 * Wraps an effect's card with its `fx-<id>` anchor so the rack can scroll to
 * and flash it. `scroll-mt` leaves room for the sticky rack.
 */
export function FxAnchor({
  id,
  className,
  children,
}: {
  id: EffectId;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      id={`fx-${id}`}
      className={cn("scroll-mt-40 rounded-card [&>section]:h-full", className)}
    >
      {children}
    </div>
  );
}
