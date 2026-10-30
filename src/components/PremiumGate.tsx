import type { ReactNode } from "react";
import { Lock, Sparkles } from "lucide-react";
import { Button } from "@/components/Button";
import { openSupporterSheet } from "@/components/SupporterSheet";
import {
  PREMIUM_FEATURES,
  useEntitlementStore,
  type PremiumFeature,
} from "@/stores/entitlement";
import { useAccountStore } from "@/stores/account";
import { FEATURE_GATING_ENABLED } from "@/config/payments";

/**
 * Feature gating is permanently off (`FEATURE_GATING_ENABLED = false`): every
 * feature is free, so `requirePremium()` always passes and `<PremiumGate>`
 * always renders its children. The plumbing stays only so the ~20 call sites
 * don't need rewriting; nothing here ever surfaces an upsell in normal use.
 */

/** Imperative guard for gated toggles/handlers. Always `true` while gating is
 *  off; callers bail out on `false`:
 *
 * ```ts
 * if (!requirePremium("SYSTEM_EQ")) return;
 * ```
 */
export function requirePremium(_feature: PremiumFeature): boolean {
  if (!FEATURE_GATING_ENABLED) return true;
  const s = useEntitlementStore.getState();
  if (s.premium || s.supporter || useAccountStore.getState().status?.isAdmin) return true;
  openSupporterSheet();
  return false;
}

interface PremiumGateProps {
  feature: PremiumFeature;
  children: ReactNode;
  /** `"inline"` (default): compact card for a settings `<Card>`. `"page"`: a
   *  taller, centred block for a whole routed view. */
  variant?: "inline" | "page";
}

/** Renders `children` — always, while gating is off. */
export function PremiumGate({ feature, children, variant = "inline" }: PremiumGateProps) {
  const premium = useEntitlementStore((s) => s.premium || s.supporter);
  const isAdmin = useAccountStore((s) => s.status?.isAdmin ?? false);
  if (!FEATURE_GATING_ENABLED || premium || isAdmin) return <>{children}</>;
  return <UpsellCard feature={feature} variant={variant} />;
}

function UpsellCard({
  feature,
  variant,
}: {
  feature: PremiumFeature;
  variant: "inline" | "page";
}) {
  return (
    <div
      className={
        variant === "page"
          ? "mx-auto flex max-w-md flex-col items-center gap-3 py-16 text-center"
          : "flex flex-col items-center gap-3 rounded-card border border-dashed border-border bg-surface/40 px-6 py-8 text-center"
      }
    >
      <div className="grid size-11 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
        <Lock className="size-5" aria-hidden="true" />
      </div>
      <p className="text-sm font-medium">{PREMIUM_FEATURES[feature]} is a Supporter feature</p>
      <Button variant="primary" onClick={() => openSupporterSheet()} className="mt-1">
        <Sparkles className="size-4" aria-hidden="true" />
        Become a Supporter
      </Button>
    </div>
  );
}
