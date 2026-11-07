import { useCallback, useEffect, useId, useRef, useState } from "react";
import { X } from "lucide-react";
import { SettingsView } from "@/features/settings/SettingsView";
import { onSettingsOpen } from "@/lib/ipc";
import { prefersReducedMotion } from "@/lib/reducedMotion";
import { cn } from "@/lib/cn";
import { closeSettings, openSettings, useSettingsDialogStore } from "./settingsDialogStore";
import { isSettingsSection, SETTINGS_SECTIONS, type SettingsSection } from "./sections";

/** Ring classes flashed on a section a deep link points at. */
const FLASH = ["ring-2", "ring-accent/60", "rounded-card"] as const;
const FLASH_MS = 1000;

/** Marks the panel so `escapeIsClaimed` can tell its own `role="dialog"` apart. */
const PANEL_ATTR = "data-settings-panel";

/** True when Escape belongs to something above or inside Settings, not to Settings. */
function escapeIsClaimed(): boolean {
  const el = document.activeElement;
  if (el instanceof HTMLElement) {
    if (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return true;
  }
  return !!document.querySelector(
    `[role="dialog"]:not([${PANEL_ATTR}]), [role="listbox"], [role="menu"], [aria-haspopup][aria-expanded="true"]`,
  );
}

/**
 * The Settings modal: a header, a section nav and the unchanged `SettingsView`,
 * inside the main window. Rendered once by `App`; the panel mounts only while
 * open so every open starts at the top.
 *
 * Layered at z-40 inside the app root so toasts (z-50, same stacking context)
 * and the body-level portals (Dialog, Plus sheet, confirm, popovers) all
 * sit above it.
 */
export function SettingsDialog() {
  const open = useSettingsDialogStore((s) => s.open);

  // The native menu / tray ask for Settings through this event.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onSettingsOpen(() => void openSettings())
      .then((un) => (cancelled ? un() : (unlisten = un)))
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return open ? <SettingsPanel /> : null;
}

function SettingsPanel() {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const section = useSettingsDialogStore((s) => s.section);
  const requestId = useSettingsDialogStore((s) => s.requestId);
  const [active, setActive] = useState<SettingsSection>(section ?? "about");
  // After a jump, the target stays highlighted until the scroll settles: a
  // section near the end can't reach the top, so the spy would otherwise pick
  // whichever section the scroll stopped on.
  const pinnedUntil = useRef(0);

  const goTo = useCallback((id: string, opts?: { flash?: boolean; smooth?: boolean }) => {
    const el = document.getElementById(id);
    if (!el || !isSettingsSection(id)) return;
    el.scrollIntoView({
      behavior: opts?.smooth && !prefersReducedMotion ? "smooth" : "auto",
      block: "start",
    });
    setActive(id);
    pinnedUntil.current = Date.now() + 1200;
    if (opts?.flash && !prefersReducedMotion) {
      el.classList.add(...FLASH);
      window.setTimeout(() => el.classList.remove(...FLASH), FLASH_MS);
    }
  }, []);

  // Deep link: the opening request jumps instantly, later requests (while
  // already open) scroll smoothly. Each request is handled once.
  const handled = useRef<number | null>(null);
  useEffect(() => {
    if (handled.current === requestId) return;
    const first = handled.current === null;
    handled.current = requestId;
    if (section) goTo(section, { flash: true, smooth: !first });
  }, [requestId, section, goTo]);

  // Focus moves into the panel; closing hands it back.
  useEffect(() => {
    const previous = document.activeElement;
    panelRef.current?.focus({ preventScroll: true });
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) {
        previous.focus({ preventScroll: true });
      }
    };
  }, []);

  // Highlight the section in view.
  useEffect(() => {
    const root = scrollRef.current;
    if (!root || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (Date.now() < pinnedUntil.current) return;
        const hit = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
        if (hit && isSettingsSection(hit.target.id)) setActive(hit.target.id);
      },
      { root, rootMargin: "0px 0px -70% 0px" },
    );
    for (const { id } of SETTINGS_SECTIONS) {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    }
    // At the very bottom the last sections never reach the top band; highlight
    // the last one whose heading is on screen instead.
    const onScroll = () => {
      if (Date.now() < pinnedUntil.current) return;
      if (root.scrollTop + root.clientHeight < root.scrollHeight - 2) return;
      const bottom = root.getBoundingClientRect().bottom;
      const visible = SETTINGS_SECTIONS.filter(({ id }) => {
        const el = document.getElementById(id);
        return el ? el.getBoundingClientRect().top < bottom - 48 : false;
      });
      const last = visible[visible.length - 1];
      if (last) setActive(last.id);
    };
    root.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      observer.disconnect();
      root.removeEventListener("scroll", onScroll);
    };
  }, []);

  // Esc / Cmd+W / Ctrl+W close Settings. Capture phase, so this sees the DOM
  // before a Dialog's own Escape handler closes it: Esc with the Plus
  // sheet open over Settings closes only the sheet.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") {
        if (!escapeIsClaimed()) closeSettings();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "w") {
        e.preventDefault();
        closeSettings();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div
      className={cn(
        "fixed inset-0 z-40 flex items-center justify-center p-4 sm:p-6",
        !prefersReducedMotion && "animate-[hm-backdrop-in_150ms_ease-out]",
      )}
    >
      <div className="absolute inset-0 bg-black/50" onClick={closeSettings} aria-hidden="true" />
      <div
        ref={panelRef}
        {...{ [PANEL_ATTR]: "" }}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="@container relative flex h-[min(840px,100%)] w-[min(820px,100%)] flex-col overflow-hidden rounded-card border border-border bg-canvas text-text shadow-2xl outline-none"
      >
        <header
          style={{ height: 52 }}
          className="flex shrink-0 items-center justify-between border-b border-border px-6"
        >
          <h2 id={titleId} className="text-sm font-medium">
            Settings
          </h2>
          <button
            type="button"
            aria-label="Close settings"
            onClick={closeSettings}
            className="text-text-faint transition-colors hover:text-text"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        </header>

        <div className="flex min-h-0 flex-1">
          <nav
            aria-label="Settings sections"
            className="hidden w-48 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-3 @2xl:flex"
          >
            {SETTINGS_SECTIONS.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                type="button"
                onClick={() => goTo(id, { smooth: true })}
                aria-current={active === id ? "true" : undefined}
                className={cn(
                  "flex items-center gap-2.5 rounded-control px-3 py-1.5 text-left text-sm transition-colors",
                  active === id
                    ? "bg-surface-overlay font-medium text-text"
                    : "text-text-muted hover:bg-surface-overlay hover:text-text",
                )}
              >
                <Icon className="size-4 shrink-0" aria-hidden="true" />
                <span className="truncate">{label}</span>
              </button>
            ))}
          </nav>

          <div ref={scrollRef} className="min-w-0 flex-1 overflow-y-auto p-6">
            <SettingsView />
          </div>
        </div>
      </div>
    </div>
  );
}
