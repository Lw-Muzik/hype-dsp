import { useState } from "react";
import type { FormEvent } from "react";
import { ArrowLeft, KeyRound, LogOut, Mail, Sparkles } from "lucide-react";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { openSupporterUpsell } from "@/components/PremiumGate";
import { useAccountStore } from "@/stores/account";
import { useEntitlementStore, usePremium } from "@/stores/entitlement";
import { ipcErrorMessage } from "@/lib/ipc";
import { PAYMENTS_ENABLED } from "@/config/payments";

const inputCls =
  "rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus-visible:border-accent";

type Flow = "none" | "attach" | "signin";
type Step = "details" | "code";

/**
 * Account — the free-tier session is always signed in (a frictionless
 * anonymous account provisioned by `SessionGate`, no wall). This card is
 * entirely optional:
 *  - attach a real email in place (sync + donation receipts), or
 *  - sign in to a different, already-existing account on a new device.
 *
 * Supporter status comes from the entitlement store, not from this account —
 * an anonymous install can be a Supporter too (Donation → Install, no email
 * required), so the badge and upsell show regardless of `emailVerified`.
 */
export function AccountCard() {
  const account = useAccountStore((s) => s.status);
  const logout = useAccountStore((s) => s.logout);
  const claimRequest = useAccountStore((s) => s.claimRequest);
  const claimVerify = useAccountStore((s) => s.claimVerify);
  const requestOtp = useAccountStore((s) => s.requestOtp);
  const verify = useAccountStore((s) => s.verify);
  const premium = usePremium();
  const entitlementLoading = useEntitlementStore((s) => s.loading);

  // Admin → exempt from purchases, full access, no upsell. Trial → premium is
  // on loan for the first N days (license.state === "trial"); after it lapses
  // premium re-locks and the upsell returns.
  const isAdmin = account?.isAdmin ?? false;
  const license = account?.license ?? null;
  const onTrial = premium && !isAdmin && license?.state === "trial";
  const daysLeft = license?.daysLeft ?? 0;

  const [flow, setFlow] = useState<Flow>("none");
  const [step, setStep] = useState<Step>("details");
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const resetFlow = () => {
    setFlow("none");
    setStep("details");
    setEmail("");
    setName("");
    setCode("");
    setError(null);
  };

  const submitDetails = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (flow === "attach") await claimRequest(email.trim());
      else await requestOtp(email.trim());
      setCode("");
      setStep("code");
    } catch (err) {
      setError(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const submitCode = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (flow === "attach") {
        await claimVerify(email.trim(), code.trim(), name.trim() || undefined);
      } else {
        await verify(email.trim(), code.trim());
      }
      resetFlow();
    } catch (err) {
      setError(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const good = "bg-success/15 text-success";
  const muted = "bg-surface-overlay text-text-muted";
  const { badgeLabel, badgeCls } = ((): { badgeLabel: string; badgeCls: string } => {
    if (isAdmin) return { badgeLabel: "Admin", badgeCls: good };
    if (entitlementLoading) return { badgeLabel: "Checking…", badgeCls: muted };
    if (onTrial)
      return {
        badgeLabel: `Trial · ${daysLeft} day${daysLeft === 1 ? "" : "s"} left`,
        badgeCls: good,
      };
    if (premium) return { badgeLabel: "Supporter", badgeCls: good };
    return { badgeLabel: "Free", badgeCls: muted };
  })();
  const premiumBadge = (
    <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${badgeCls}`}>
      {badgeLabel}
    </span>
  );

  // Payments off ⇒ there are no plans to distinguish, so the whole Plan row
  // (badge + explainer) and the purchase CTA come out. See
  // `@/config/payments`.
  const showPlan = PAYMENTS_ENABLED;
  // Purchase CTA appears for free users and trial users (convert early), but
  // never for the admin (exempt) or an established Supporter/licensed account.
  const showUpsell = PAYMENTS_ENABLED && !isAdmin && (!premium || onTrial);
  // A one-line explainer under the plan row for the two special states.
  const planNote = !PAYMENTS_ENABLED
    ? null
    : isAdmin
      ? "Full access to every feature — no purchase needed."
      : onTrial
        ? "You're on a free trial of the premium suite. When it ends, premium features return to the free tier — core playback stays free."
        : null;

  if (!account?.authenticated) {
    return (
      <Card title="Account" icon={KeyRound}>
        <p className="py-2 text-sm text-text-muted">Starting your session…</p>
      </Card>
    );
  }

  if (flow !== "none") {
    return (
      <Card title={flow === "attach" ? "Attach email" : "Sign in"} icon={Mail}>
        {step === "details" ? (
          <form onSubmit={(e) => void submitDetails(e)} className="flex flex-col gap-3">
            <p className="text-sm text-text-muted">
              {flow === "attach"
                ? "Attach an email to sync your library and settings across devices and get donation receipts. We'll send a code — no password needed."
                : "Enter the email of an existing HypeMuzik account and we'll send a sign-in code."}
            </p>
            <input
              className={inputCls}
              type="email"
              autoComplete="email"
              autoFocus
              required
              placeholder="Email"
              aria-label="Email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
            {error && <p className="text-sm text-danger">{error}</p>}
            <div className="flex items-center gap-2">
              <Button variant="primary" type="submit" disabled={busy}>
                {busy ? "Sending…" : "Send code"}
              </Button>
              <Button variant="ghost" type="button" onClick={resetFlow}>
                Cancel
              </Button>
            </div>
          </form>
        ) : (
          <form onSubmit={(e) => void submitCode(e)} className="flex flex-col gap-3">
            <p className="text-sm text-text-muted">
              Enter the 6-digit code sent to <span className="text-text">{email}</span>.
            </p>
            {flow === "attach" && (
              <input
                className={inputCls}
                placeholder="Name (optional)"
                aria-label="Name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            )}
            <input
              className={`${inputCls} text-center text-lg tracking-[0.5em] tabular-nums`}
              inputMode="numeric"
              autoComplete="one-time-code"
              autoFocus
              required
              maxLength={6}
              placeholder="000000"
              aria-label="Verification code"
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
            />
            {error && <p className="text-sm text-danger">{error}</p>}
            <div className="flex items-center gap-2">
              <Button variant="primary" type="submit" disabled={busy || code.length < 6}>
                {busy ? "Verifying…" : "Verify"}
              </Button>
              <Button
                variant="ghost"
                type="button"
                onClick={() => {
                  setStep("details");
                  setError(null);
                }}
              >
                <ArrowLeft className="size-4" aria-hidden="true" />
                Back
              </Button>
            </div>
          </form>
        )}
      </Card>
    );
  }

  return (
    <Card title="Account" icon={KeyRound}>
      <div className="flex flex-col gap-3">
        {account.emailVerified ? (
          <div className="flex flex-col gap-1">
            <div className="divide-y divide-border">
              {account.email && <InfoRow label="Email" value={account.email} />}
              {account.name && <InfoRow label="Name" value={account.name} />}
              {showPlan && (
                <div className="flex items-center justify-between py-2 text-sm">
                  <span className="text-text-muted">Plan</span>
                  {premiumBadge}
                </div>
              )}
            </div>
            {planNote && <p className="text-xs text-text-muted">{planNote}</p>}
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <p className="text-sm text-text-muted">
              You’re using HypeMuzik anonymously — there’s nothing to sign in
              with yet. Attach an email to sync your library and settings
              across devices
              {PAYMENTS_ENABLED ? " and get donation receipts." : "."}
            </p>
            {showPlan && (
              <div className="flex items-center gap-2 text-sm">
                <span className="text-text-muted">Plan</span>
                {premiumBadge}
              </div>
            )}
            {planNote && <p className="text-xs text-text-muted">{planNote}</p>}
          </div>
        )}

        <div className="mt-1 flex flex-wrap items-center justify-end gap-2">
          {showUpsell && (
            <Button variant="primary" onClick={() => openSupporterUpsell()}>
              <Sparkles className="size-4" aria-hidden="true" />
              Become a Supporter
            </Button>
          )}
          {!account.emailVerified ? (
            <>
              <Button variant="secondary" onClick={() => setFlow("attach")}>
                <Mail className="size-4" aria-hidden="true" />
                Attach email
              </Button>
              <Button variant="ghost" onClick={() => setFlow("signin")}>
                Sign in instead
              </Button>
            </>
          ) : (
            <Button variant="ghost" onClick={() => void logout()}>
              <LogOut className="size-4" aria-hidden="true" />
              Sign out
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-2 text-sm">
      <span className="text-text-muted">{label}</span>
      <span className="font-medium tabular-nums">{value}</span>
    </div>
  );
}
