import { useState } from "react";
import { Radio, Tv } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { cn } from "@/lib/cn";
import { RadioPanel } from "./RadioPanel";
import { TvPanel } from "./TvPanel";

type Kind = "radio" | "tv";

const KINDS: { id: Kind; label: string; icon: typeof Radio }[] = [
  { id: "radio", label: "Radio", icon: Radio },
  { id: "tv", label: "TV", icon: Tv },
];

/**
 * The Stations hub: one destination hosting two kinds of live media. Radio
 * streams as audio through the DSP engine (unchanged); TV plays as video in a
 * native mpv window. A segmented toggle switches between them; each kind owns
 * its own browse/favorites state below.
 */
export function StationsView() {
  const route = routeById("stations");
  const [kind, setKind] = useState<Kind>("radio");

  return (
    <div className="mx-auto flex h-full w-full max-w-4xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="flex items-center gap-1 self-start rounded-control border border-border bg-surface-raised p-1">
        {KINDS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            onClick={() => setKind(id)}
            className={cn(
              "flex items-center gap-2 rounded-[7px] px-4 py-1.5 text-sm font-medium transition-colors",
              kind === id
                ? "bg-surface-overlay text-text"
                : "text-text-muted hover:text-text",
            )}
          >
            <Icon className="size-4" aria-hidden="true" />
            {label}
          </button>
        ))}
      </div>

      {/* Both panels stay mounted so switching kinds preserves each one's
          browse mode, search results and scroll position. */}
      <div className={cn("min-h-0 flex-1", kind !== "radio" && "hidden")}>
        <RadioPanel />
      </div>
      <div className={cn("min-h-0 flex-1", kind !== "tv" && "hidden")}>
        <TvPanel active={kind === "tv"} />
      </div>
    </div>
  );
}
