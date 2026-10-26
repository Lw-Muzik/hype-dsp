import { describe, expect, it } from "vitest";
import { ROUTES } from "@/app/routes";
import {
  EQUALIZER_ROUTES,
  modeFromSystemEq,
  routeForMode,
  visibleRoutes,
} from "./appMode";

describe("appMode", () => {
  it("derives the mode from system EQ", () => {
    expect(modeFromSystemEq(true)).toBe("equalizer");
    expect(modeFromSystemEq(false)).toBe("player");
  });

  it("Player mode shows today's sidebar list unchanged", () => {
    expect(visibleRoutes("player").map((r) => r.id)).toEqual(
      ROUTES.filter((r) => !r.hidden).map((r) => r.id),
    );
  });

  it("Equalizer mode shows exactly the three audio-tuning routes, in order", () => {
    expect(visibleRoutes("equalizer").map((r) => r.id)).toEqual([
      "equalizer",
      "enhancer",
      "visuals",
    ]);
    expect(EQUALIZER_ROUTES).toEqual(["equalizer", "enhancer", "visuals"]);
  });

  it("Stems lives in Player mode only: it separates the app's player, not system audio", () => {
    expect(visibleRoutes("player").map((r) => r.id)).toContain("stems");
    expect(visibleRoutes("equalizer").map((r) => r.id)).not.toContain("stems");
  });

  it("routeForMode keeps visible routes and falls back otherwise", () => {
    expect(routeForMode("equalizer", "stems")).toBe("equalizer");
    expect(routeForMode("player", "stems")).toBe("stems");
    expect(routeForMode("equalizer", "player")).toBe("equalizer");
    expect(routeForMode("equalizer", "mixer")).toBe("equalizer");
    expect(routeForMode("player", "mixer")).toBe("mixer");
    expect(routeForMode("player", "equalizer")).toBe("equalizer");
  });
});
