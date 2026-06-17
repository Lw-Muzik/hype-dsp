import { useState } from "react";
import { Sparkles } from "lucide-react";
import { useUiStore } from "@/stores/ui";
import { Dialog } from "@/components/Dialog";
import { Button } from "@/components/Button";
import { ipcErrorMessage, licenseActivate } from "@/lib/ipc";
import { cn } from "@/lib/cn";

/** Trial countdown / expired banner with a (mock) activation dialog. */
export function TrialBanner() {
  const license = useUiStore((s) => s.license);
  const setLicense = useUiStore((s) => s.setLicense);
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  if (!license || license.kind === "licensed") return null;
  const expired = license.kind === "expired";

  const activate = () => {
    setError(null);
    licenseActivate(key)
      .then((status) => {
        setLicense(status);
        setOpen(false);
        setKey("");
      })
      .catch((e) => setError(ipcErrorMessage(e)));
  };

  return (
    <>
      <div
        className={cn(
          "flex items-center justify-between gap-3 border-b px-5 py-2 text-sm",
          expired
            ? "border-danger/30 bg-danger/10"
            : "border-border bg-surface-raised",
        )}
      >
        <span
          className={cn(
            "flex items-center gap-2",
            expired ? "text-danger" : "text-text-muted",
          )}
        >
          <Sparkles className="size-4" aria-hidden="true" />
          {expired
            ? "Your trial has expired."
            : `Trial — ${license.daysLeft} day${license.daysLeft === 1 ? "" : "s"} left.`}
        </span>
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="rounded-control border border-border px-2.5 py-1 text-xs transition-colors hover:bg-surface-overlay"
        >
          Activate
        </button>
      </div>

      <Dialog open={open} onClose={() => setOpen(false)} title="Activate HypeMuzik">
        <div className="flex flex-col gap-3">
          <p className="text-sm text-text-muted">
            Enter your license key.{" "}
            <span className="text-text-faint">
              (Mock — no real activation server; any non-empty key works.)
            </span>
          </p>
          <input
            autoFocus
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") activate();
            }}
            placeholder="License key"
            aria-label="License key"
            className="rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none focus-visible:border-accent"
          />
          {error && <p className="text-sm text-danger">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button variant="primary" onClick={activate} disabled={!key.trim()}>
              Activate
            </Button>
          </div>
        </div>
      </Dialog>
    </>
  );
}
