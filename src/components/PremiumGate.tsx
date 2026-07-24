import { useEffect, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { Loader2, Lock, Sparkles } from "lucide-react";
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

/** The checkout modal's own progression: pick an amount → wait for Paddle to
 *  confirm the browser payment → celebrate. */
type CheckoutPhase = "select" | "pending" | "success";

/** While waiting for a payment to confirm, re-check entitlement this often (on
 *  top of the focus-triggered refresh), giving up — and pointing at Restore —
 *  after the budget, so a webhook that never lands doesn't spin forever. */
const CONFIRM_POLL_MS = 4000;
const CONFIRM_POLL_BUDGET = 30; // ~2 minutes

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

/**
 * Fires a single celebratory toast whenever the account transitions into
 * Supporter mid-session — covering unlocks that land while the upsell modal
 * isn't showing its success view (a late webhook, a restore from the Account
 * card, a cross-device sign-in). Armed only after the first entitlement check
 * settles, so the cache→server hydration on launch (which legitimately flips
 * false→true for an existing supporter) never reads as a fresh unlock.
 */
function useSupporterUnlockToast(): void {
  const premium = usePremium();
  const loading = useEntitlementStore((s) => s.loading);
  const prev = useRef(premium);
  const armed = useRef(false);
  useEffect(() => {
    if (!armed.current) {
      if (!loading) {
        armed.current = true;
        prev.current = premium;
      }
      return;
    }
    if (!prev.current && premium) {
      toast.success("You're a Supporter — thank you! Everything's unlocked.");
    }
    prev.current = premium;
  }, [premium, loading]);
}

/** The modal every upsell CTA opens. Mount exactly once (in `App.tsx`). */
export function SupporterUpsellModal() {
  const open = useUpsellStore((s) => s.open);
  const feature = useUpsellStore((s) => s.feature);
  const close = useUpsellStore((s) => s.close);
  const premium = usePremium();

  // Also carries the app-wide unlock acknowledgement — this component is
  // always mounted, so it's the natural home for the transition watcher.
  useSupporterUnlockToast();

  return (
    <Dialog
      open={open}
      onClose={close}
      title={premium ? "Supporter" : "Become a Supporter"}
    >
      {/* Remount per open so the checkout phase always starts fresh. */}
      {open && <UpsellBody feature={feature} onClose={close} />}
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
  const premium = usePremium();
  const [amount, setAmount] = useState<number>(10);
  const [custom, setCustom] = useState("");
  const [checkingOut, setCheckingOut] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [phase, setPhase] = useState<CheckoutPhase>("select");
  const [stalled, setStalled] = useState(false);

  // Entitlement flipped to premium (via the poll below, the focus refresh, or
  // a Restore) → celebrate, from whatever phase we're in.
  useEffect(() => {
    if (premium && phase !== "success") setPhase("success");
  }, [premium, phase]);

  // While confirming, poll entitlement on top of SessionGate's focus refresh,
  // and give up gracefully after the budget so a payment whose webhook never
  // arrives (e.g. before the backend is on HTTPS) points the user at Restore
  // instead of spinning forever.
  useEffect(() => {
    if (phase !== "pending") return;
    let ticks = 0;
    const id = window.setInterval(() => {
      ticks += 1;
      void useEntitlementStore.getState().refresh();
      if (ticks >= CONFIRM_POLL_BUDGET) {
        setStalled(true);
        window.clearInterval(id);
      }
    }, CONFIRM_POLL_MS);
    return () => window.clearInterval(id);
  }, [phase]);

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
      // Keep the modal open and switch to a "confirming…" view rather than
      // closing — the user shouldn't be left wondering whether it worked. That
      // view polls entitlement and either unlocks or points at Restore itself.
      setPhase("pending");
    } catch (err) {
      toast.error(ipcErrorMessage(err));
    } finally {
      setCheckingOut(false);
    }
  }

  if (phase === "success") return <SupporterSuccess onClose={onClose} />;
  if (phase === "pending")
    return <SupporterPending stalled={stalled} onClose={onClose} />;

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
        // The refresh flips entitlement to premium, which the central
        // useSupporterUnlockToast acknowledges (and drives the modal's success
        // view) — so we deliberately don't toast a second thank-you here.
        void useEntitlementStore.getState().refresh();
      } else {
        toast.info("No Supporter checkout found for that email.");
      }
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

/** Shown after the browser checkout is opened: we're waiting for Paddle to
 *  confirm the payment (which flips entitlement to premium). Offers Restore as
 *  an escape hatch, auto-surfaced once the wait has stalled. */
function SupporterPending({
  stalled,
  onClose,
}: {
  stalled: boolean;
  onClose: () => void;
}) {
  const [restoreOpen, setRestoreOpen] = useState(false);
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-brand-from/15 text-brand-from">
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        </div>
        <div className="flex flex-col gap-1 text-sm">
          <p className="font-medium text-text">Confirming your payment…</p>
          <p className="text-text-muted">
            Finish checkout in your browser if you haven&apos;t yet. Premium
            unlocks here automatically once your payment is confirmed — usually
            just a few seconds.
          </p>
        </div>
      </div>

      {stalled && (
        <p className="text-sm text-text-muted">
          This is taking longer than usual. If you&apos;ve already completed
          payment, restore it with your receipt email:
        </p>
      )}

      <RestoreSupporter
        open={stalled || restoreOpen}
        onToggle={() => setRestoreOpen((v) => !v)}
      />

      <div className="flex justify-end">
        <Button variant="ghost" onClick={onClose}>
          Dismiss
        </Button>
      </div>
    </div>
  );
}

/** Shown the moment entitlement flips to Supporter — the explicit thank-you the
 *  silent badge-flip was missing. */
function SupporterSuccess({ onClose }: { onClose: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 py-4 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
        <Sparkles className="size-7" aria-hidden="true" />
      </div>
      <div className="flex flex-col gap-1">
        <p className="text-base font-semibold">You&apos;re a Supporter — thank you!</p>
        <p className="text-sm text-text-muted">
          Everything&apos;s unlocked: the full Advanced DSP suite, stem
          separation, and cloud music. We genuinely appreciate the support.
        </p>
      </div>
      <Button variant="primary" onClick={onClose} className="mt-1">
        Done
      </Button>
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
