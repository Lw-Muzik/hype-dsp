import { useState } from "react";
import { Lock } from "lucide-react";
import { Button } from "@/components/Button";
import { useAccountStore } from "@/stores/account";
import type { LicenseInfo } from "@/lib/types";

/** Shown when the user is signed in but the server denies access (trial over or
 *  admin-blocked). The app's features are not rendered behind this. */
export function TrialEndedScreen({
  license,
  email,
}: {
  license: LicenseInfo | null;
  email: string | null;
}) {
  const refresh = useAccountStore((s) => s.refresh);
  const logout = useAccountStore((s) => s.logout);
  const [checking, setChecking] = useState(false);
  const blocked = license?.state === "blocked";

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-surface p-4 text-text">
      <div className="w-full max-w-md rounded-2xl border border-danger/30 bg-surface-raised p-8 text-center shadow-lg">
        <div className="mx-auto mb-4 grid size-12 place-items-center rounded-xl bg-danger/15 text-danger">
          <Lock className="size-6" aria-hidden="true" />
        </div>
        <h1 className="text-xl font-semibold">
          {blocked ? "Access paused" : "Your trial has ended"}
        </h1>
        <p className="mt-2 text-sm text-text-muted">
          {blocked
            ? "An administrator has paused access to this account. Reach out if you think this is a mistake."
            : "Thanks for trying HypeMuzik. Your free trial is over — get in touch to upgrade and keep your sound."}
        </p>
        {email && (
          <p className="mt-3 text-xs text-text-faint">Signed in as {email}</p>
        )}
        <div className="mt-6 flex justify-center gap-2">
          <Button
            variant="secondary"
            disabled={checking}
            onClick={async () => {
              setChecking(true);
              await refresh();
              setChecking(false);
            }}
          >
            {checking ? "Checking…" : "Check again"}
          </Button>
          <Button variant="ghost" onClick={() => void logout()}>
            Sign out
          </Button>
        </div>
      </div>
    </div>
  );
}
