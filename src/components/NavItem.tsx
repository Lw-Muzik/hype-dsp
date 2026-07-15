import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/cn";

interface NavItemProps {
  icon: LucideIcon;
  label: string;
  active: boolean;
  collapsed: boolean;
  onClick: () => void;
}

/** A single sidebar navigation entry. */
export function NavItem({
  icon: Icon,
  label,
  active,
  collapsed,
  onClick,
}: NavItemProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      title={collapsed ? label : undefined}
      className={cn(
        "relative flex items-center gap-3 rounded-control px-3 py-2 text-sm font-medium transition-colors",
        collapsed && "justify-center",
        active
          ? "bg-surface-overlay text-text hover:cursor-pointer"
          : "text-text-muted hover:bg-surface-raised hover:text-text hover:cursor-pointer",
      )}
    >
      {active && (
        <span
          className="absolute left-0 h-5 w-0.5 rounded-full bg-accent"
          aria-hidden="true"
        />
      )}
      <Icon className="size-[18px] shrink-0" aria-hidden="true" />
      {!collapsed && <span className="truncate">{label}</span>}
    </button>
  );
}
