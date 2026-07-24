import { useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { Lock, Sparkles } from "lucide-react";
import { create } from "zustand";
import { Button } from "@/components/Button";
import { Dialog } from "@/components/Dialog";
import { toast } from "@/stores/toast";
import {
  accountSupportCheckout,
  accountSupportRestore,
  ipcErrorMessage,
  openExternalUrl,
} from "@/lib/ipc";
import {
  PREMIUM_FEATURES,
  useEntitlementStore,
  usePremium,
  type PremiumFeature,
} from "@/stores/entitlement";

/** Quick-pick Supporter amounts, in whole dollars. */
const QUICK_AMOUNTS = [5, 10, 25] as const;

const inputCls =
  "w-full rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus-visible:border-accent";

// --------------------------------------------------------- imperative upsell

interface UpsellStore {
  open: boolean;
  feature: PremiumFeature | null;
  show: (feature?: PremiumFeature) => void;
  close: () => void;
}

/** Backs {@link requirePremium}'s modal and every "Become a Supporter" CTA in
 *  the app (declarative gate cards, the imperative guard, and the Settings
 *  Account card) — they all funnel through this one store, which
 *  `<SupporterUpsellModal>` (mounted once near the app root) renders. */
const useUpsellStore = create<UpsellStore>((set) => ({
  open: false,
  feature: null,
  show: (feature) => set({ open: true, feature: feature ?? null }),
  close: () => set({ open: false, feature: null }),
}));

/**
 * The single entry point every "Become a Supporter" CTA calls: opens the
 * shared upsell/checkout modal, optionally naming the feature that triggered
 * it. A generic entry point (e.g. the Settings Account card, which isn't
 * gating any one feature) calls this with no argument.
 */
export function openSupporterUpsell(feature?: PremiumFeature): void {
  useUpsellStore.getState().show(feature);
}

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
  openSupporterUpsell(feature);
  return false;
}

/** The modal every upsell CTA opens. Mount exactly once (in `App.tsx`). */
export function SupporterUpsellModal() {
  const open = useUpsellStore((s) => s.open);
  const feature = useUpsellStore((s) => s.feature);
  const close = useUpsellStore((s) => s.close);

  return (
    <Dialog open={open} onClose={close} title="Become a Supporter">
      <UpsellBody feature={feature} onClose={close} />
    </Dialog>
  );
}

function UpsellBody({
  feature,
  onClose,
}: {
  feature: PremiumFeature | null;
  onClose: () => void;
}) {
  const [amount, setAmount] = useState<number>(10);
  const [custom, setCustom] = useState("");
  const [checkingOut, setCheckingOut] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);

  // A non-empty custom amount always wins over the quick picks — including
  // when it doesn't parse, so a user who typed garbage never silently
  // checks out at whatever quick amount was last selected.
  const trimmedCustom = custom.trim();
  const usingCustom = trimmedCustom !== "";
  const selectedAmount = usingCustom ? Number(trimmedCustom) : amount;
  const validAmount = Number.isFinite(selectedAmount) && selectedAmount >= 1;

  async function handleCheckout() {
    if (!validAmount || checkingOut) return;
    setCheckingOut(true);
    try {
      const checkoutUrl = await accountSupportCheckout(selectedAmount);
      await openExternalUrl(checkoutUrl);
      toast.info(
        "Complete your Supporter checkout in the browser — premium unlocks automatically.",
      );
      onClose();
      // The user may return to a focused window (SessionGate already refreshes
      // then) or may still be mid-checkout — kick one now too so a fast
      // checkout unlocks without waiting for a focus event.
      void useEntitlementStore.getState().refresh();
    } catch (err) {
      toast.error(ipcErrorMessage(err));
    } finally {
      setCheckingOut(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
          <Sparkles className="size-4" aria-hidden="true" />
        </div>
        <p className="text-sm text-text-muted">
          {feature && (
            <>
              <span className="font-medium text-text">{PREMIUM_FEATURES[feature]}</span>{" "}
              is a Supporter feature.{" "}
            </>
          )}
          Support HypeMuzik to unlock the full Advanced DSP suite, stem
          separation, and cloud music.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex gap-2" role="group" aria-label="Amount">
          {QUICK_AMOUNTS.map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => {
                setAmount(value);
                setCustom("");
              }}
              className={`flex-1 rounded-control border px-3 py-2 text-sm font-medium transition-colors ${
                !usingCustom && amount === value
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border text-text-muted hover:bg-surface-raised"
              }`}
            >
              ${value}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-2 text-sm text-text-muted">
          <span>$</span>
          <input
            type="number"
            min={1}
            inputMode="decimal"
            placeholder="Custom amount"
            aria-label="Custom amount"
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            className={inputCls}
          />
        </label>
      </div>

      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Maybe later
        </Button>
        <Button
          variant="primary"
          disabled={!validAmount || checkingOut}
          onClick={() => void handleCheckout()}
        >
          <Sparkles className="size-4" aria-hidden="true" />
          {checkingOut ? "Opening checkout…" : validAmount ? `Support $${selectedAmount}` : "Support"}
        </Button>
      </div>

      <RestoreSupporter open={restoreOpen} onToggle={() => setRestoreOpen((v) => !v)} />
    </div>
  );
}

/** "Already a Supporter?" affordance in the upsell modal's footer — a receipt
 *  email restores the entitlement onto this install without a full sign-in
 *  (a reinstall, or supporting from a second machine). Collapsed by default
 *  so it doesn't compete with the primary checkout CTA. */
function RestoreSupporter({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);

  async function handleRestore(e: FormEvent) {
    e.preventDefault();
    const trimmed = email.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      const restored = await accountSupportRestore(trimmed);
      if (restored) {
        toast.success("Supporter restored — thanks again for your support!");
      } else {
        toast.info("No Supporter checkout found for that email.");
      }
      void useEntitlementStore.getState().refresh();
    } catch (err) {
      toast.error(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  if (!open) {
    return (
      <div className="border-t border-border pt-3">
        <button
          type="button"
          onClick={onToggle}
          className="text-sm text-text-muted underline-offset-2 hover:text-text hover:underline"
        >
          Already a Supporter? Restore purchase
        </button>
      </div>
    );
  }

  return (
    <form
      onSubmit={(e) => void handleRestore(e)}
      className="flex items-center gap-2 border-t border-border pt-3"
    >
      <input
        type="email"
        required
        autoFocus
        placeholder="Receipt email"
        aria-label="Receipt email"
        value={email}
        onChange={(e) => setEmail(e.target.value)}
        className={inputCls}
      />
      <Button variant="secondary" type="submit" disabled={busy}>
        {busy ? "Checking…" : "Restore"}
      </Button>
    </form>
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
      <Button
        variant="primary"
        onClick={() => openSupporterUpsell(feature)}
        className="mt-1"
      >
        <Sparkles className="size-4" aria-hidden="true" />
        Become a Supporter
      </Button>
    </div>
  );
}
