import { beforeEach, describe, expect, it, vi } from "vitest";

const requirePremium = vi.fn((_f?: string) => true);
vi.mock("@/components/PremiumGate", () => ({ requirePremium: (f: string) => requirePremium(f) }));
vi.mock("@/lib/ipc", () => ({ ipcErrorMessage: (e: unknown) => String(e) }));

const calls: string[] = [];
const setScope = vi.fn(async (_s: unknown) => {
  calls.push("scope");
});
const eq = {
  enabled: false,
  enable: vi.fn(async () => {
    calls.push("enable");
  }),
};
vi.mock("@/stores/engine", () => ({
  useEngineStore: {
    getState: () => ({
      setSystemEqScope: setScope,
      state: { systemEqScope: { mode: "only", apps: ["a"] } },
    }),
  },
}));
vi.mock("@/stores/systemEq", () => ({ useSystemEqStore: { getState: () => eq } }));

import { applySystemEqScope, scopeSummary, setScopeMode, toggleScopeApp } from "./systemEqScope";

beforeEach(() => {
  calls.length = 0;
  eq.enabled = false;
  vi.clearAllMocks();
  requirePremium.mockReturnValue(true);
});

describe("systemEqScope", () => {
  it("commits the scope before re-enabling, only when on", async () => {
    eq.enabled = true;
    expect(await applySystemEqScope({ mode: "all", apps: [] })).toBeNull();
    expect(calls).toEqual(["scope", "enable"]);
  });

  it("does not re-enable when system EQ is off", async () => {
    await applySystemEqScope({ mode: "all", apps: [] });
    expect(calls).toEqual(["scope"]);
  });

  it("surfaces a commit failure and skips the re-enable", async () => {
    eq.enabled = true;
    setScope.mockRejectedValueOnce("boom");
    expect(await applySystemEqScope({ mode: "all", apps: [] })).toBe("boom");
    expect(eq.enable).not.toHaveBeenCalled();
  });

  it("premium refusal blocks mode and app changes", async () => {
    requirePremium.mockReturnValue(false);
    await setScopeMode("except");
    await toggleScopeApp("b");
    expect(requirePremium).toHaveBeenCalledWith("PER_APP_EQ");
    expect(setScope).not.toHaveBeenCalled();
  });

  it("toggles apps in and out, keeping the mode", async () => {
    await toggleScopeApp("b");
    expect(setScope).toHaveBeenLastCalledWith({ mode: "only", apps: ["a", "b"] });
    await toggleScopeApp("a");
    expect(setScope).toHaveBeenLastCalledWith({ mode: "only", apps: [] });
    await setScopeMode("except");
    expect(setScope).toHaveBeenLastCalledWith({ mode: "except", apps: ["a"] });
  });

  it("summarises the scope", () => {
    expect(scopeSummary({ mode: "all", apps: [] })).toBe("All apps");
    expect(scopeSummary({ mode: "only", apps: ["a"] })).toBe("Only 1 app");
    expect(scopeSummary({ mode: "except", apps: ["a", "b"] })).toBe("All except 2 apps");
  });
});
