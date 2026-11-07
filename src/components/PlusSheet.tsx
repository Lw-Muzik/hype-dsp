import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Check, Loader2, Sparkles } from "lucide-react";
import { create } from "zustand";
import { Button } from "@/components/Button";
import { Dialog } from "@/components/Dialog";
import { SupporterBadge } from "@/components/SupporterBadge";
import { toast } from "@/stores/toast";
import {
  accountSupportCheckout,
  accountSupportRestore,
  ipcErrorMessage,
  openExternalUrl,
} from "@/lib/ipc";
import {
  PREMIUM_FEATURES,
  trialDaysLeft,
  trialEndsToday,
  trialHasEnded,
  useEntitlementStore,
  useSupporter,
} from "@/stores/entitlement";
import { PLUS_FEATURES, SUPPORTER_ENABLED } from "@/config/payments";

/** The six features Plus unlocks, as plain labels (kept in sync with
 *  `PLUS_FEATURES`). */
const PLUS_FEATURE_LABELS = [...PLUS_FEATURES].map((f) => PREMIUM_FEATURES[f]);

/** The sheet's own progression: offer -> wait for Paddle to confirm the
 *  browser payment -> thank-you. An account that already has Plus opens
 *  straight on "owned". */
type Phase = "select" | "pending" | "success" | "owned";

/** While waiting for a payment to confirm, re-check entitlement this often (on
 *  top of the focus-triggered refresh), giving up — and pointing at Restore —
 *  after the budget, so a webhook that never lands doesn't spin forever. */
const CONFIRM_POLL_MS = 4000;
const CONFIRM_POLL_BUDGET = 30; // ~2 minutes

const inputCls =
  "w-full rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus-visible:border-accent";

// ------------------------------------------------------- imperative opening

interface SheetStore {
  open: boolean;
  show: () => void;
  close: () => void;
}

const useSheetStore = create<SheetStore>((set) => ({
  open: false,
  show: () => set({ open: true }),
  close: () => set({ open: false }),
}));

/** The one entry point every "Get Plus" button calls. Mounted once per window
 *  via {@link PlusSheet}. Opened by an explicit click (Settings -> Account) or
 *  when a locked Plus feature is used. */
export function openPlusSheet(): void {
  if (!SUPPORTER_ENABLED) return;
  useSheetStore.getState().show();
}

/** Close the sheet programmatically (used by tests and window teardown). */
export function closePlusSheet(): void {
  useSheetStore.getState().close();
}

/**
 * Fires a single thank-you toast whenever the account transitions into
 * Plus mid-session (a late webhook, a restore, a sign-in elsewhere).
 * Armed only after the first entitlement check settles, so the cache->server
 * hydration on launch never reads as a fresh purchase.
 */
function useSupporterThanks(enabled: boolean): void {
  const supporter = useSupporter();
  const loading = useEntitlementStore((s) => s.loading);
  const prev = useRef(supporter);
  const armed = useRef(false);
  useEffect(() => {
    if (!SUPPORTER_ENABLED || !enabled) return;
    if (!armed.current) {
      if (!loading) {
        armed.current = true;
        prev.current = supporter;
      }
      return;
    }
    if (!prev.current && supporter) {
      toast.success("HypeMuzik Plus is unlocked — thank you!");
    }
    prev.current = supporter;
  }, [supporter, loading, enabled]);
}

/**
 * The "HypeMuzik Plus" sheet. Mount once per window: `App.tsx` in the main
 * window, and any secondary window (with `thanksToast={false}`, since the main
 * window already toasts and both windows would otherwise).
 */
export function PlusSheet({ thanksToast = true }: { thanksToast?: boolean } = {}) {
  const open = useSheetStore((s) => s.open);
  const close = useSheetStore((s) => s.close);

  useSupporterThanks(thanksToast);

  if (!SUPPORTER_ENABLED) return null;

  return (
    <Dialog
      open={open}
      onClose={close}
      title="HypeMuzik Plus"
    >
      {/* Remount per open so the phase always starts fresh. */}
      {open && <SheetBody onClose={close} />}
    </Dialog>
  );
}

function SheetBody({ onClose }: { onClose: () => void }) {
  const supporter = useSupporter();
  const [checkingOut, setCheckingOut] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [phase, setPhase] = useState<Phase>(supporter ? "owned" : "select");
  const [stalled, setStalled] = useState(false);

  // Entitlement flipped to Plus while the sheet was open (poll, focus refresh,
  // or Restore) -> thank-you, from whatever phase we're in.
  useEffect(() => {
    if (supporter && phase !== "success" && phase !== "owned") setPhase("success");
  }, [supporter, phase]);

  // While confirming, poll entitlement and give up gracefully after the
  // budget, pointing at Restore instead of spinning forever.
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

  async function handleCheckout() {
    if (checkingOut) return;
    setCheckingOut(true);
    setError(null);
    try {
      // No amount: Plus is a fixed-price licence; the backend charges it.
      const checkoutUrl = await accountSupportCheckout();
      await openExternalUrl(checkoutUrl);
      setPhase("pending");
    } catch (err) {
      // Shown plainly in the sheet — never a silent failure.
      setError(ipcErrorMessage(err) || "Checkout couldn't be started. Please try again.");
    } finally {
      setCheckingOut(false);
    }
  }

  if (phase === "success") return <PlusOwned justBought onClose={onClose} />;
  if (phase === "owned") return <PlusOwned onClose={onClose} />;
  if (phase === "pending") return <PlusPending stalled={stalled} onClose={onClose} />;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
          <Sparkles className="size-4" aria-hidden="true" />
        </div>
        <div className="flex flex-col gap-1 text-sm">
          <p className="font-semibold text-text">US$5 · one-time · yours for life</p>
          <p className="text-text-muted">
            Unlock the advanced audio tools. No subscription. Everything else in
            HypeMuzik stays free.
          </p>
        </div>
      </div>

      <TrialLine />

      <FeatureList />

      <p className="text-xs text-text-muted">
        Your licence is tied to the email you use at checkout. Refundable within
        14 days. Plus buyers also get a <SupporterBadge className="align-middle" />{" "}
        badge in the app.
      </p>

      {error && (
        <p
          role="alert"
          className="rounded-control border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger"
        >
          {error}
        </p>
      )}

      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Maybe later
        </Button>
        <Button
          variant="primary"
          disabled={checkingOut}
          onClick={() => void handleCheckout()}
        >
          <Sparkles className="size-4" aria-hidden="true" />
          {checkingOut ? "Opening checkout…" : "Get Plus — $5"}
        </Button>
      </div>

      <RestorePlus open={restoreOpen} onToggle={() => setRestoreOpen((v) => !v)} />
    </div>
  );
}

/** Trial status: "Plus trial · N days left" while a trial runs (the buy button
 *  stays so they can buy early), or a one-line note once it has ended. */
function TrialLine() {
  const premium = useEntitlementStore((s) => s.premium);
  const supporter = useSupporter();
  const trialEndsAt = useEntitlementStore((s) => s.trialEndsAt);
  if (supporter) return null;
  const days = trialDaysLeft(trialEndsAt);
  if (premium && days !== null) {
    const label = trialEndsToday(trialEndsAt)
      ? "Plus trial · ends today"
      : `Plus trial · ${days} ${days === 1 ? "day" : "days"} left`;
    return (
      <p
        data-testid="plus-trial"
        className="rounded-control border border-accent/40 bg-accent-muted px-3 py-1.5 text-sm font-medium text-accent-strong"
      >
        {label}
      </p>
    );
  }
  if (!premium && trialHasEnded(trialEndsAt)) {
    return (
      <p data-testid="plus-trial" className="text-sm text-text-muted">
        Your 7-day Plus trial has ended.
      </p>
    );
  }
  return null;
}

/** The six Plus features, as a plain checklist. */
function FeatureList() {
  return (
    <ul aria-label="Included with Plus" className="grid grid-cols-1 gap-1.5 text-sm sm:grid-cols-2">
      {PLUS_FEATURE_LABELS.map((label) => (
        <li key={label} className="flex items-center gap-2">
          <Check className="size-4 shrink-0 text-accent" aria-hidden="true" />
          <span>{label}</span>
        </li>
      ))}
    </ul>
  );
}

/** "Already bought Plus?" — the checkout email links a purchase (made here or
 *  on hypemuzik.com) to this install. Collapsed by default so it doesn't
 *  compete with the primary CTA. */
function RestorePlus({
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
        // The refresh flips `supporter`, which the central thank-you watcher
        // acknowledges (and drives the sheet's success view).
        void useEntitlementStore.getState().refresh();
      } else {
        toast.info("No HypeMuzik Plus purchase found for that email.");
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
          Already bought Plus? Restore purchase
        </button>
      </div>
    );
  }

  return (
    <form
      onSubmit={(e) => void handleRestore(e)}
      className="flex flex-col gap-2 border-t border-border pt-3"
    >
      <p className="text-xs text-text-muted">
        Enter the email you used at checkout to link your purchase to this app.
      </p>
      <div className="flex items-center gap-2">
        <input
          type="email"
          required
          autoFocus
          placeholder="Checkout email"
          aria-label="Checkout email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          className={inputCls}
        />
        <Button variant="secondary" type="submit" disabled={busy}>
          {busy ? "Checking…" : "Restore"}
        </Button>
      </div>
    </form>
  );
}

/** Shown after the browser checkout is opened: waiting for the payment to be
 *  confirmed (which flips `supporter`). Restore is the escape hatch, surfaced
 *  automatically once the wait has stalled. */
function PlusPending({
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
            Finish checkout in your browser if you haven&apos;t yet. Plus
            unlocks here automatically once the payment is confirmed — usually
            just a few seconds.
          </p>
        </div>
      </div>

      {stalled && (
        <p className="text-sm text-text-muted">
          This is taking longer than usual. If you&apos;ve already completed
          payment, restore it with your checkout email:
        </p>
      )}

      <RestorePlus
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

/** The "You have Plus" state: shown when the account already owns Plus, and
 *  (with `justBought`) as the thank-you right after a purchase confirms. */
function PlusOwned({
  justBought = false,
  onClose,
}: {
  justBought?: boolean;
  onClose: () => void;
}) {
  return (
    <div className="flex flex-col items-center gap-4 py-4 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
        <Sparkles className="size-7" aria-hidden="true" />
      </div>
      <div className="flex flex-col items-center gap-2">
        <p className="text-base font-semibold">
          {justBought ? "You have Plus — thank you!" : "You have Plus"}
        </p>
        <SupporterBadge />
        <p className="text-sm text-text-muted">
          Every Plus feature is unlocked on this account, for life.
        </p>
      </div>
      <FeatureList />
      <Button variant="primary" onClick={onClose} className="mt-1">
        Done
      </Button>
    </div>
  );
}
