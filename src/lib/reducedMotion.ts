/**
 * Whether the user asked for less motion.
 *
 * Read once at import, matching the existing call sites. The CSS blanket rule
 * in styles/index.css already collapses animation/transition timing; this is
 * for JS-driven motion that CSS can't reach.
 */
export const prefersReducedMotion: boolean = (() => {
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
})();
