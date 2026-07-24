import type { ReactNode } from "react";
import { Lock, Sparkles } from "lucide-react";
import { create } from "zustand";
import { Button } from "@/components/Button";
import { Dialog } from "@/components/Dialog";
import { toast } from "@/stores/toast";
import {
  PREMIUM_FEATURES,
  useEntitlementStore,
  usePremium,
  type PremiumFeature,
} from "@/stores/entitlement";

/**
 * Placeholder for the real checkout (wired to Paddle in a follow-on phase —
 * see the design spec's non-goals). Every upsell CTA in the app calls this one
 * function, so swapping it for the real flow later is a single-place change.
 */
export function openSupporterCheckout(): void {
  toast.info("Supporter checkout is coming soon — thanks for your patience!");
}

// --------------------------------------------------------- imperative upsell

interface UpsellStore {
  feature: PremiumFeature | null;
  open: (feature: PremiumFeature) => void;
  close: () => void;
}

/** Backs {@link requirePremium}'s modal and `<SupporterUpsellModal>` (mounted
 *  once near the app root, alongside `<Toaster>`). */
const useUpsellStore = create<UpsellStore>((set) => ({
  feature: null,
  open: (feature) => set({ feature }),
  close: () => set({ feature: null }),
}));

/**
 * Imperative guard for gated toggles/handlers that aren't wrapping JSX (e.g.
 * "Enable system-wide EQ"): returns whether `feature` is unlocked right now,
 * and if not, surfaces the "Become a Supporter" upsell as a modal instead of
 * silently no-op'ing. Callers should bail out on `false`:
 *
 * ```ts
 * if (!requirePremium("SYSTEM_EQ")) return;
 * ```
 */
export function requirePremium(feature: PremiumFeature): boolean {
  if (useEntitlementStore.getState().premium) return true;
  useUpsellStore.getState().open(feature);
  return false;
}

/** The modal `requirePremium` opens. Mount exactly once (in `App.tsx`). */
export function SupporterUpsellModal() {
  const feature = useUpsellStore((s) => s.feature);
  const close = useUpsellStore((s) => s.close);

  return (
    <Dialog open={feature !== null} onClose={close} title="Supporter feature">
      {feature && <UpsellBody feature={feature} onClose={close} />}
    </Dialog>
  );
}

function UpsellBody({
  feature,
  onClose,
}: {
  feature: PremiumFeature;
  onClose: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
          <Sparkles className="size-4" aria-hidden="true" />
        </div>
        <p className="text-sm text-text-muted">
          <span className="font-medium text-text">{PREMIUM_FEATURES[feature]}</span>{" "}
          is a Supporter feature. Support HypeMuzik to unlock the full Advanced
          DSP suite, stem separation, and cloud music.
        </p>
      </div>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Maybe later
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            openSupporterCheckout();
            onClose();
          }}
        >
          <Sparkles className="size-4" aria-hidden="true" />
          Become a Supporter
        </Button>
      </div>
    </div>
  );
}

// -------------------------------------------------------------- declarative

interface PremiumGateProps {
  feature: PremiumFeature;
  children: ReactNode;
  /** `"inline"` (default): a compact upsell card sized for sitting inside a
   *  settings `<Card>`. `"page"`: a taller, centred block for a whole routed
   *  view (Stems, Mixer). */
  variant?: "inline" | "page";
}

/**
 * Renders `children` when the account is a Supporter, otherwise a
 * "Become a Supporter" upsell in the same place — the feature simply isn't
 * there for a free user, rather than being visible-but-broken.
 */
export function PremiumGate({ feature, children, variant = "inline" }: PremiumGateProps) {
  const premium = usePremium();
  if (premium) return <>{children}</>;
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
      <div className="flex flex-col gap-1">
        <p className="text-sm font-medium">{PREMIUM_FEATURES[feature]} is a Supporter feature</p>
        <p className="text-sm text-text-muted">
          Support HypeMuzik to unlock the full Advanced DSP suite, stem
          separation, and cloud music.
        </p>
      </div>
      <Button variant="primary" onClick={openSupporterCheckout} className="mt-1">
        <Sparkles className="size-4" aria-hidden="true" />
        Become a Supporter
      </Button>
    </div>
  );
}
