import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

interface CardProps {
  title: string;
  icon?: LucideIcon;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** A titled content panel — the app's standard surface for grouped content. */
export function Card({ title, icon: Icon, actions, children, className }: CardProps) {
  return (
    <section
      className={cn(
        "rounded-card border border-border bg-surface-raised",
        className,
      )}
    >
      <header className="flex items-center gap-2 border-b border-border px-4 py-3">
        {Icon && <Icon className="size-4 text-text-muted" aria-hidden="true" />}
        <h3 className="text-sm font-medium">{title}</h3>
        {actions && <div className="ml-auto">{actions}</div>}
      </header>
      <div className="p-4">{children}</div>
    </section>
  );
}
