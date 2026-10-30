import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Loader2, Sparkles } from "lucide-react";
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
import { useEntitlementStore, useSupporter } from "@/stores/entitlement";
import {
  SUPPORTER_DEFAULT_AMOUNT,
  SUPPORTER_ENABLED,
  SUPPORTER_MIN_AMOUNT,
  SUPPORTER_QUICK_AMOUNTS,
} from "@/config/payments";

/** The sheet's own progression: pick an amount -> wait for Paddle to confirm
 *  the browser payment -> thank-you. */
type Phase = "select" | "pending" | "success";

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

/** The one entry point every "Become a Supporter" button calls. Mounted once
 *  per window via {@link SupporterSheet}. Nothing in normal use calls this —
 *  only an explicit click (Settings -> Account). */
export function openSupporterSheet(): void {
  if (!SUPPORTER_ENABLED) return;
  useSheetStore.getState().show();
}

/**
 * Fires a single thank-you toast whenever the account transitions into
 * Supporter mid-session (a late webhook, a restore, a sign-in elsewhere).
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
      toast.success("You're a Supporter — thank you!");
    }
    prev.current = supporter;
  }, [supporter, loading, enabled]);
}

/**
 * The "Become a Supporter" sheet. Mount once per window: `App.tsx` in the main
 * window, and the Settings window (with `thanksToast={false}`, since the main
 * window already toasts and both windows would otherwise).
 */
export function SupporterSheet({ thanksToast = true }: { thanksToast?: boolean } = {}) {
  const open = useSheetStore((s) => s.open);
  const close = useSheetStore((s) => s.close);
  const supporter = useSupporter();

  useSupporterThanks(thanksToast);

  if (!SUPPORTER_ENABLED) return null;

  return (
    <Dialog
      open={open}
      onClose={close}
      title={supporter ? "Supporter" : "Become a Supporter"}
    >
      {/* Remount per open so the phase always starts fresh. */}
      {open && <SheetBody onClose={close} />}
    </Dialog>
  );
}

function SheetBody({ onClose }: { onClose: () => void }) {
  const supporter = useSupporter();
  const [amount, setAmount] = useState<number>(SUPPORTER_DEFAULT_AMOUNT);
  const [custom, setCustom] = useState("");
  const [checkingOut, setCheckingOut] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [phase, setPhase] = useState<Phase>("select");
  const [stalled, setStalled] = useState(false);

  // Entitlement flipped to supporter (poll, focus refresh, or Restore) ->
  // thank-you, from whatever phase we're in.
  useEffect(() => {
    if (supporter && phase !== "success") setPhase("success");
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

  // A non-empty custom amount always wins over the quick picks — including
  // when it doesn't parse, so garbage never silently checks out at a quick pick.
  const trimmedCustom = custom.trim();
  const usingCustom = trimmedCustom !== "";
  const selectedAmount = usingCustom ? Number(trimmedCustom) : amount;
  const validAmount =
    Number.isFinite(selectedAmount) && selectedAmount >= SUPPORTER_MIN_AMOUNT;

  async function handleCheckout() {
    if (!validAmount || checkingOut) return;
    setCheckingOut(true);
    setError(null);
    try {
      const checkoutUrl = await accountSupportCheckout(selectedAmount);
      await openExternalUrl(checkoutUrl);
      setPhase("pending");
    } catch (err) {
      // Shown plainly in the sheet (checkout is expected to fail until the
      // payment provider approves the account) — never a silent failure.
      setError(ipcErrorMessage(err) || "Checkout couldn't be started. Please try again.");
    } finally {
      setCheckingOut(false);
    }
  }

  if (phase === "success") return <SupporterSuccess onClose={onClose} />;
  if (phase === "pending") return <SupporterPending stalled={stalled} onClose={onClose} />;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
          <Sparkles className="size-4" aria-hidden="true" />
        </div>
        <div className="flex flex-col gap-2 text-sm text-text-muted">
          <p>
            HypeMuzik is free, with every feature unlocked — and it stays that way.
            Supporting is optional: pay what you want and show it with a{" "}
            <SupporterBadge className="align-middle" /> badge in the app.
          </p>
          <p>Nothing is ever locked behind it.</p>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex gap-2" role="group" aria-label="Amount">
          {SUPPORTER_QUICK_AMOUNTS.map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => {
                setAmount(value);
                setCustom("");
                setError(null);
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
            min={SUPPORTER_MIN_AMOUNT}
            inputMode="decimal"
            placeholder={`Custom amount (min $${SUPPORTER_MIN_AMOUNT})`}
            aria-label="Custom amount"
            value={custom}
            onChange={(e) => {
              setCustom(e.target.value);
              setError(null);
            }}
            className={inputCls}
          />
        </label>
        {usingCustom && !validAmount && (
          <p className="text-xs text-text-muted">
            The minimum is ${SUPPORTER_MIN_AMOUNT}.
          </p>
        )}
      </div>

      <p className="text-xs text-text-muted">
        One-time payment, not a subscription. Refundable within 14 days.
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

/** "Already a Supporter?" — a receipt email links a purchase (made here or on
 *  hypemuzik.com/support) to this install. Collapsed by default so it doesn't
 *  compete with the primary CTA. */
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
        // The refresh flips `supporter`, which the central thank-you watcher
        // acknowledges (and drives the sheet's success view).
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
      className="flex flex-col gap-2 border-t border-border pt-3"
    >
      <p className="text-xs text-text-muted">
        Bought on hypemuzik.com/support? Enter the email on your receipt to link
        it to this app.
      </p>
      <div className="flex items-center gap-2">
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
      </div>
    </form>
  );
}

/** Shown after the browser checkout is opened: waiting for the payment to be
 *  confirmed (which flips `supporter`). Restore is the escape hatch, surfaced
 *  automatically once the wait has stalled. */
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
            Finish checkout in your browser if you haven&apos;t yet. Your
            Supporter badge appears here automatically once the payment is
            confirmed — usually just a few seconds.
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

function SupporterSuccess({ onClose }: { onClose: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 py-4 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
        <Sparkles className="size-7" aria-hidden="true" />
      </div>
      <div className="flex flex-col items-center gap-2">
        <p className="text-base font-semibold">You&apos;re a Supporter — thank you!</p>
        <SupporterBadge />
        <p className="text-sm text-text-muted">
          Your badge is now showing in the app. We genuinely appreciate it.
        </p>
      </div>
      <Button variant="primary" onClick={onClose} className="mt-1">
        Done
      </Button>
    </div>
  );
}
