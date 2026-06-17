import { CheckCircle2, CircleAlert, Info, X } from "lucide-react";
import { useToastStore } from "@/stores/toast";
import type { ToastKind } from "@/stores/toast";
import { cn } from "@/lib/cn";

const icons: Record<ToastKind, typeof Info> = {
  error: CircleAlert,
  success: CheckCircle2,
  info: Info,
};

const accents: Record<ToastKind, string> = {
  error: "text-danger",
  success: "text-success",
  info: "text-accent-strong",
};

/** Bottom-right toast stack. An ARIA live region announces new toasts. */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  return (
    <div
      aria-live="polite"
      aria-atomic="false"
      className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-80 flex-col gap-2"
    >
      {toasts.map((t) => {
        const Icon = icons[t.kind];
        return (
          <div
            key={t.id}
            role="status"
            className="pointer-events-auto flex items-start gap-2.5 rounded-card border border-border bg-surface-overlay px-3.5 py-3 text-sm shadow-lg"
          >
            <Icon
              className={cn("mt-0.5 size-4 shrink-0", accents[t.kind])}
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1">{t.message}</span>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={() => dismiss(t.id)}
              className="text-text-faint transition-colors hover:text-text"
            >
              <X className="size-3.5" aria-hidden="true" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
