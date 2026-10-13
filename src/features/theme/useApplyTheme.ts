import { useEffect } from "react";
import { useThemeStore, watchPrefersDark } from "@/stores/theme";

/**
 * Applies the theme to this window's document: follows the OS light/dark
 * preference and writes `data-theme`, the backdrop-blur variable and the boot
 * background. Every window that renders UI calls this once (main and Settings).
 */
export function useApplyTheme(): void {
  const resolved = useThemeStore((s) => s.resolved);
  const blur = useThemeStore((s) => s.blur);
  const setPrefersDark = useThemeStore((s) => s.setPrefersDark);

  useEffect(() => watchPrefersDark(setPrefersDark), [setPrefersDark]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = resolved;
    // The slider retargets this one variable, so dragging it never re-renders
    // the image — only the blur filter's input changes.
    root.style.setProperty("--hm-backdrop-blur", `${blur}px`);
    // The boot script painted a guess; once React owns the theme, keep them
    // agreeing so a later theme change repaints the base too.
    root.style.setProperty("--hm-boot-bg", resolved === "light" ? "#f4f5f7" : "#0a0b0e");
  }, [resolved, blur]);
}
