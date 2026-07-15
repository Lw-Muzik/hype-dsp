import { useState } from "react";
import { AudioLines, ArrowLeft } from "lucide-react";
import { Button } from "@/components/Button";
import { useAccountStore } from "@/stores/account";
import { ipcErrorMessage } from "@/lib/ipc";

const inputCls =
  "rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus-visible:border-accent";

/** Passwordless sign in / create account: email (+ name) → emailed code. */
export function AuthScreen() {
  const signup = useAccountStore((s) => s.signup);
  const requestOtp = useAccountStore((s) => s.requestOtp);
  const verify = useAccountStore((s) => s.verify);

  const [step, setStep] = useState<"details" | "code">("details");
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function sendCode(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (mode === "signup") await signup(email, name.trim() || undefined);
      else await requestOtp(email);
      setCode("");
      setStep("code");
    } catch (err) {
      setError(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function submitCode(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      // On success the auth gate re-renders into the app.
      await verify(email, code.trim());
    } catch (err) {
      setError(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function resend() {
    setError(null);
    try {
      // The account exists by now, so a plain request sends a fresh code.
      await requestOtp(email);
    } catch (err) {
      setError(ipcErrorMessage(err));
    }
  }

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-surface p-4 text-text">
      <div className="w-full max-w-sm rounded-2xl border border-border bg-surface-raised p-7 shadow-lg">
        <div className="mb-5">
          <div className="mb-3 grid size-11 place-items-center rounded-xl bg-gradient-to-br from-brand-from to-brand-to text-on-brand">
            <AudioLines className="size-5" aria-hidden="true" />
          </div>
          {step === "details" ? (
            <>
              <h1 className="text-xl font-semibold">
                {mode === "login" ? "Welcome back" : "Create your account"}
              </h1>
              <p className="text-sm text-text-muted">
                {mode === "login"
                  ? "Enter your email and we'll send a sign-in code."
                  : "Start your free trial — we'll email you a code, no password needed."}
              </p>
            </>
          ) : (
            <>
              <h1 className="text-xl font-semibold">Check your email</h1>
              <p className="text-sm text-text-muted">
                Enter the 6-digit code sent to{" "}
                <span className="text-text">{email}</span>.
              </p>
            </>
          )}
        </div>

        {step === "details" ? (
          <form onSubmit={sendCode} className="flex flex-col gap-3">
            {mode === "signup" && (
              <input
                className={inputCls}
                placeholder="Name (optional)"
                aria-label="Name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            )}
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
            <Button variant="primary" type="submit" disabled={busy} className="mt-1">
              {busy ? "Sending…" : "Send code"}
            </Button>
          </form>
        ) : (
          <form onSubmit={submitCode} className="flex flex-col gap-3">
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
              onChange={(e) =>
                setCode(e.target.value.replace(/\D/g, "").slice(0, 6))
              }
            />
            {error && <p className="text-sm text-danger">{error}</p>}
            <Button
              variant="primary"
              type="submit"
              disabled={busy || code.length < 6}
              className="mt-1"
            >
              {busy ? "Verifying…" : "Verify & continue"}
            </Button>
            <div className="flex items-center justify-between text-xs text-text-muted">
              <button
                type="button"
                onClick={() => {
                  setStep("details");
                  setError(null);
                }}
                className="flex items-center gap-1 transition-colors hover:text-text"
              >
                <ArrowLeft className="size-3" aria-hidden="true" />
                Change email
              </button>
              <button
                type="button"
                onClick={() => void resend()}
                className="transition-colors hover:text-text"
              >
                Resend code
              </button>
            </div>
          </form>
        )}

        {step === "details" && (
          <button
            type="button"
            onClick={() => {
              setMode(mode === "login" ? "signup" : "login");
              setError(null);
            }}
            className="mt-4 w-full text-center text-xs text-text-muted transition-colors hover:text-text"
          >
            {mode === "login"
              ? "Don’t have an account? Sign up"
              : "Already have an account? Sign in"}
          </button>
        )}
      </div>
    </div>
  );
}
