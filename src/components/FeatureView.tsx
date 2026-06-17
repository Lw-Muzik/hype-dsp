import type { NavRoute } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";

/**
 * Honest empty state for a view whose functionality lands in a later phase.
 * It shows what the view will do and its build phase — never fabricated data.
 */
export function FeatureView({ route }: { route: NavRoute }) {
  const Icon = route.icon;
  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />
      <div className="flex min-h-[360px] flex-col items-center justify-center rounded-card border border-dashed border-border bg-surface-raised/40 p-10 text-center">
        <div className="mb-4 flex size-14 items-center justify-center rounded-full bg-surface-overlay text-text-faint">
          <Icon className="size-7" aria-hidden="true" />
        </div>
        <p className="text-sm font-medium text-text">
          {route.label} arrives in Phase {route.phase}
        </p>
        <p className="mt-1.5 max-w-sm text-sm text-text-muted">{route.tagline}</p>
        <span className="mt-5 inline-flex items-center gap-1.5 rounded-full border border-border bg-surface px-3 py-1 text-xs text-text-faint">
          <span className="size-1.5 rounded-full bg-warning" aria-hidden="true" />
          Scaffolded — not yet functional
        </span>
      </div>
    </div>
  );
}
