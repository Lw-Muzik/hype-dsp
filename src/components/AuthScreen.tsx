import { useState } from "react";
import { AudioLines } from "lucide-react";
import { Button } from "@/components/Button";
import { useAccountStore } from "@/stores/account";
import { ipcErrorMessage } from "@/lib/ipc";

const inputCls =
  "rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus-visible:border-accent";

/** Sign in / create account — shown by the gate until the user authenticates. */
export function AuthScreen() {
  const login = useAccountStore((s) => s.login);
  const signup = useAccountStore((s) => s.signup);

  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (mode === "login") await login(email, password);
      else await signup(email, password, name.trim() || undefined);
    } catch (err) {
      setError(ipcErrorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-surface p-4 text-text">
      <div className="w-full max-w-sm rounded-2xl border border-border bg-surface-raised p-7 shadow-lg">
        <div className="mb-5">
          <div className="mb-3 grid size-11 place-items-center rounded-xl bg-accent text-text">
            <AudioLines className="size-5" aria-hidden="true" />
          </div>
          <h1 className="text-xl font-semibold">
            {mode === "login" ? "Welcome back" : "Create your account"}
          </h1>
          <p className="text-sm text-text-muted">
            {mode === "login"
              ? "Sign in to use HypeMuzik."
              : "Start your free trial of HypeMuzik."}
          </p>
        </div>

        <form onSubmit={submit} className="flex flex-col gap-3">
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
          <input
            className={inputCls}
            type="password"
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            required
            minLength={8}
            placeholder="Password"
            aria-label="Password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          {error && <p className="text-sm text-danger">{error}</p>}
          <Button variant="primary" type="submit" disabled={busy} className="mt-1">
            {busy
              ? "Please wait…"
              : mode === "login"
                ? "Sign in"
                : "Create account"}
          </Button>
        </form>

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
      </div>
    </div>
  );
}
