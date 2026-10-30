import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toaster } from "@/components/Toaster";
import { SupporterSheet } from "@/components/SupporterSheet";
import { SettingsView } from "@/features/settings/SettingsView";
import { useApplyTheme } from "@/features/theme/useApplyTheme";
import { onSettingsSection, type SettingsSection } from "@/lib/ipc";
import { TITLEBAR_INSET } from "@/lib/platform";
import { prefersReducedMotion } from "@/lib/reducedMotion";
import { cn } from "@/lib/cn";
import { isSettingsSection, SETTINGS_SECTIONS } from "./sections";

/** Ring classes flashed on a section a deep link points at. */
const FLASH = ["ring-2", "ring-accent/60", "rounded-card"] as const;
const FLASH_MS = 1000;

/** True when Escape belongs to something inside the window, not to the window. */
function escapeIsClaimed(): boolean {
  const el = document.activeElement;
  if (el instanceof HTMLElement) {
    if (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return true;
  }
  return !!document.querySelector(
    '[role="dialog"], [role="listbox"], [role="menu"], [aria-haspopup][aria-expanded="true"]',
  );
}

const closeWindow = () => {
  void getCurrentWindow()
    .close()
    .catch(() => {});
};

/**
 * Root of the Settings child window: a draggable title strip, a section nav
 * and the unchanged `SettingsView`. Runs none of the main window's global
 * effects (see `main.tsx`); its stores are kept in step by `windowSyncSetup`.
 */
export function SettingsWindow() {
  useApplyTheme();

  const scrollRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<SettingsSection>("about");
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

  // Deep link from the URL hash, then from a later `settings_open(section)`.
  useEffect(() => {
    const hash = decodeURIComponent(window.location.hash.replace(/^#/, ""));
    if (hash) goTo(hash);
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onSettingsSection((section) => goTo(section, { flash: true, smooth: true }))
      .then((un) => (cancelled ? un() : (unlisten = un)))
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [goTo]);

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

  // Esc / Cmd+W / Ctrl+W close the window.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") {
        if (!escapeIsClaimed()) closeWindow();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "w") {
        e.preventDefault();
        closeWindow();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-surface text-text">
      {/* Title strip and drag handle. On macOS the close light overlays its
          top-left corner, so the title sits below the traffic-light inset. */}
      <header
        data-tauri-drag-region="deep"
        style={{ height: 52 + TITLEBAR_INSET, paddingTop: TITLEBAR_INSET }}
        className="flex shrink-0 items-center border-b border-border px-6"
      >
        <h1 className="text-sm font-medium">Settings</h1>
      </header>

      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Settings sections"
          className="hidden w-48 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-3 min-[720px]:flex"
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

      <Toaster />
      <SupporterSheet thanksToast={false} />
    </div>
  );
}
