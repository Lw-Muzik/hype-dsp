import { ROUTES, type NavRoute } from "@/app/routes";
import type { Route } from "@/stores/ui";

/** Player = the app's own library/player; Equalizer = system-wide audio tuning. */
export type AppMode = "player" | "equalizer";

/** The mode is derived from the persisted system-wide EQ intent. */
export const modeFromSystemEq = (enabled: boolean): AppMode =>
  enabled ? "equalizer" : "player";

/**
 * Routes shown in Equalizer mode, in sidebar order. No Stems: it separates
 * the app's own player, not system-wide audio, so it stays in Player mode.
 */
export const EQUALIZER_ROUTES: readonly Route[] = [
  "equalizer",
  "enhancer",
  "visuals",
];

const shown = (r: NavRoute): boolean => r.group === "main" && !r.hidden;

/** Sidebar entries for a mode. */
export function visibleRoutes(mode: AppMode): NavRoute[] {
  if (mode === "player") return ROUTES.filter(shown);
  return EQUALIZER_ROUTES.map((id) => ROUTES.find((r) => r.id === id)).filter(
    (r): r is NavRoute => !!r && shown(r),
  );
}

/** `route` if it is visible in `mode`, else the mode's home route. */
export function routeForMode(mode: AppMode, route: Route): Route {
  if (visibleRoutes(mode).some((r) => r.id === route)) return route;
  return mode === "equalizer" ? "equalizer" : "player";
}
