import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Lock, Sparkles } from "lucide-react";
import { Button } from "@/components/Button";
import { openPlusSheet } from "@/components/PlusSheet";
import {
  PREMIUM_FEATURES,
  markTrialEndedNoticeSeen,
  trialEndedNoticeSeen,
  trialHasEnded,
  useEntitlementStore,
  type PremiumFeature,
} from "@/stores/entitlement";
import { useAccountStore } from "@/stores/account";
import { PLUS_FEATURES } from "@/config/payments";

/**
 * Per-feature gating. Only the features in `PLUS_FEATURES` need HypeMuzik
 * Plus; everything else (system EQ, AutoEQ, stems, cloud, ...) always passes.
 * A Plus feature passes when the account is entitled: backend `premium`
 * (admin / trial / licence), `supporter` (a Plus purchase) or the local admin
 * check. Entitlement is cached offline, so it is answered synchronously.
 */

/** Whether the current account may use Plus features right now. */
export function isPlusEntitled(): boolean {
  const s = useEntitlementStore.getState();
  return s.premium || s.supporter || (useAccountStore.getState().status?.isAdmin ?? false);
}

/** Reactive {@link isPlusEntitled}. */
export function usePlusEntitled(): boolean {
  const entitled = useEntitlementStore((s) => s.premium || s.supporter);
  const isAdmin = useAccountStore((s) => s.status?.isAdmin ?? false);
  return entitled || isAdmin;
}

/** Whether `feature` is currently locked for this account (reactive). */
export function usePlusLocked(feature: PremiumFeature): boolean {
  const entitled = usePlusEntitled();
  return PLUS_FEATURES.has(feature) && !entitled;
}

/** Imperative guard for gated toggles/handlers. Free features always pass; a
 *  Plus feature passes only when entitled, otherwise the Plus sheet opens and
 *  the caller bails out on `false`:
 *
 * ```ts
 * if (!requirePremium("SURROUND_3D")) return;
 * ```
 */
export function requirePremium(feature: PremiumFeature): boolean {
  if (!PLUS_FEATURES.has(feature)) return true;
  if (isPlusEntitled()) return true;
  openPlusSheet();
  return false;
}

interface PremiumGateProps {
  feature: PremiumFeature;
  children: ReactNode;
  /** `"inline"` (default): compact card for a settings `<Card>`. `"page"`: a
   *  taller, centred block for a whole routed view. */
  variant?: "inline" | "page";
}

/** Renders `children`, or the Plus upsell when `feature` is locked. */
export function PremiumGate({ feature, children, variant = "inline" }: PremiumGateProps) {
  const locked = usePlusLocked(feature);
  if (!locked) return <>{children}</>;
  return <UpsellCard feature={feature} variant={variant} />;
}

function UpsellCard({
  feature,
  variant,
}: {
  feature: PremiumFeature;
  variant: "inline" | "page";
}) {
  // The first time a locked feature is hit after the trial ended, say so once;
  // afterwards the normal copy. Never shown on launch: this card only exists
  // where a locked feature is.
  const trialEndsAt = useEntitlementStore((s) => s.trialEndsAt);
  const [trialEnded] = useState(
    () => trialHasEnded(trialEndsAt) && !trialEndedNoticeSeen(trialEndsAt),
  );
  useEffect(() => {
    if (trialEnded) markTrialEndedNoticeSeen(trialEndsAt);
  }, [trialEnded, trialEndsAt]);
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
      <p className="text-sm font-medium">
        {trialEnded
          ? "Your 7-day Plus trial has ended"
          : `${PREMIUM_FEATURES[feature]} is part of HypeMuzik Plus`}
      </p>
      <Button variant="primary" onClick={() => openPlusSheet()} className="mt-1">
        <Sparkles className="size-4" aria-hidden="true" />
        Get Plus
      </Button>
    </div>
  );
}
