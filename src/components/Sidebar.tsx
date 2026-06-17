import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { ROUTES } from "@/app/routes";
import { useUiStore } from "@/stores/ui";
import { NavItem } from "@/components/NavItem";
import { Logo } from "@/components/Logo";
import { cn } from "@/lib/cn";

/** Persistent left navigation: brand, primary destinations, system, collapse. */
export function Sidebar() {
  const route = useUiStore((s) => s.route);
  const setRoute = useUiStore((s) => s.setRoute);
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);

  const main = ROUTES.filter((r) => r.group === "main");
  const system = ROUTES.filter((r) => r.group === "system");

  return (
    <aside
      className={cn(
        "flex h-full shrink-0 flex-col border-r border-border bg-surface-raised transition-[width] duration-200",
        collapsed ? "w-[68px]" : "w-60",
      )}
    >
      <div
        className={cn(
          "flex h-14 items-center gap-2.5 px-4",
          collapsed && "justify-center px-0",
        )}
      >
        <Logo />
        {!collapsed && (
          <span className="text-[15px] font-medium tracking-tight">
            HypeMuzik
          </span>
        )}
      </div>

      <nav
        aria-label="Primary"
        className="flex flex-1 flex-col gap-1 px-3 pt-2"
      >
        {main.map((r) => (
          <NavItem
            key={r.id}
            icon={r.icon}
            label={r.label}
            active={route === r.id}
            collapsed={collapsed}
            onClick={() => setRoute(r.id)}
          />
        ))}
      </nav>

      <div className="flex flex-col gap-1 px-3 pb-1">
        {system.map((r) => (
          <NavItem
            key={r.id}
            icon={r.icon}
            label={r.label}
            active={route === r.id}
            collapsed={collapsed}
            onClick={() => setRoute(r.id)}
          />
        ))}
      </div>

      <div className="px-3 pb-3 pt-1">
        <button
          type="button"
          onClick={toggleSidebar}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          className={cn(
            "flex w-full items-center gap-3 rounded-control px-3 py-2 text-sm text-text-faint transition-colors hover:bg-surface-overlay hover:text-text-muted",
            collapsed && "justify-center",
          )}
        >
          {collapsed ? (
            <PanelLeftOpen className="size-[18px]" aria-hidden="true" />
          ) : (
            <>
              <PanelLeftClose className="size-[18px]" aria-hidden="true" />
              <span>Collapse</span>
            </>
          )}
        </button>
      </div>
    </aside>
  );
}
