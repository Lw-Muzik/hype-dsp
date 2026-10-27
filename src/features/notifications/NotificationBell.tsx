import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Bell, ExternalLink } from "lucide-react";
import {
  notificationsList,
  notificationsMarkAllRead,
  notificationsOpen,
  onNotificationsUpdated,
  type AppNotification,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";

const WINDOW_MS = 14 * 24 * 60 * 60 * 1000;

/** Items from the last 14 days, newest first. Exported for tests. */
export function recent(items: AppNotification[], now = Date.now()): AppNotification[] {
  return items
    .filter((n) => {
      const t = Date.parse(n.createdAt);
      return Number.isNaN(t) || now - t <= WINDOW_MS;
    })
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

function ago(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const m = Math.max(0, Math.round((now - t) / 60_000));
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

/**
 * Top-bar bell for admin broadcasts. A gold dot means something is unread. The
 * popover lists the last 14 days; opening an item marks it read, reports
 * `opened`, and (if it has a link) opens it in the default browser. All of that
 * happens in the `notifications_open` command.
 */
export function NotificationBell() {
  const [items, setItems] = useState<AppNotification[]>([]);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelId = useId();

  const refresh = useCallback(() => {
    notificationsList()
      .then(setItems)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    let un: (() => void) | undefined;
    let dead = false;
    onNotificationsUpdated(refresh)
      .then((f) => (dead ? f() : (un = f)))
      .catch(() => {});
    return () => {
      dead = true;
      un?.();
    };
  }, [refresh]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const list = recent(items);
  const unread = list.filter((n) => !n.read).length;

  const openItem = (n: AppNotification) => {
    // Optimistic: the dot clears at once; the command returns the truth.
    setItems((cur) => cur.map((c) => (c.id === n.id ? { ...c, read: true } : c)));
    notificationsOpen(n.id)
      .then(setItems)
      .catch(() => {});
  };

  const markAll = () => {
    notificationsMarkAllRead()
      .then(setItems)
      .catch(() => {});
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-label={unread > 0 ? `Notifications, ${unread} unread` : "Notifications"}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        title="Notifications"
        onClick={() => setOpen((o) => !o)}
        className="relative grid size-8 place-items-center rounded-control text-text-faint transition-colors hover:bg-surface-overlay hover:text-text"
      >
        <Bell className="size-4" aria-hidden="true" />
        {unread > 0 && (
          <span
            data-testid="unread-dot"
            className="absolute right-1.5 top-1.5 size-2 rounded-full bg-accent ring-2 ring-surface"
          />
        )}
      </button>

      {open && (
        <div
          id={panelId}
          role="dialog"
          aria-label="Notifications"
          className="hm-pop absolute right-0 top-full z-50 mt-2 w-80 overflow-hidden rounded-control border border-border-strong bg-surface-overlay shadow-2xl ring-1 ring-black/40"
          style={{ transformOrigin: "top right" }}
        >
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <p className="font-display text-sm font-medium">Notifications</p>
            {unread > 0 && (
              <button
                type="button"
                onClick={markAll}
                className="text-xs text-text-muted transition-colors hover:text-accent"
              >
                Mark all read
              </button>
            )}
          </div>

          {list.length === 0 ? (
            <p className="px-4 py-8 text-center text-sm text-text-muted">
              Nothing new from Hype Muzik.
            </p>
          ) : (
            <ul className="max-h-96 divide-y divide-border overflow-y-auto">
              {list.map((n) => (
                <li key={n.id}>
                  <button
                    type="button"
                    onClick={() => openItem(n)}
                    className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-raised"
                  >
                    <span
                      aria-hidden="true"
                      className={cn(
                        "mt-1.5 size-2 shrink-0 rounded-full",
                        n.read ? "bg-transparent" : "bg-accent",
                      )}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline justify-between gap-2">
                        <span
                          className={cn(
                            "truncate font-display text-sm",
                            n.read ? "font-normal text-text-muted" : "font-medium text-text",
                          )}
                        >
                          {n.title}
                        </span>
                        <span className="shrink-0 text-xs tabular-nums text-text-faint">
                          {ago(n.createdAt)}
                        </span>
                      </span>
                      <span className="mt-0.5 line-clamp-3 block text-xs text-text-muted">
                        {n.body}
                      </span>
                      {n.link && (
                        <span className="mt-1.5 inline-flex items-center gap-1 text-xs text-accent">
                          <ExternalLink className="size-3" aria-hidden="true" />
                          Open link
                        </span>
                      )}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
