// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/features/mode/systemEqScope", () => ({
  applySystemEqScope: vi.fn().mockResolvedValue(null),
}));
vi.mock("@/components/PlusSheet", () => ({ openPlusSheet: vi.fn() }));
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    engineSetSurround3d: vi.fn().mockResolvedValue(undefined),
    engineSetRoom: vi.fn().mockResolvedValue(undefined),
    engineSetConvolver: vi.fn().mockResolvedValue(undefined),
  };
});

import { enforcePlus, readSuppressed } from "@/features/plus/plusEnforcement";
import { applySystemEqScope } from "@/features/mode/systemEqScope";
import { engineSetConvolver, engineSetRoom, engineSetSurround3d } from "@/lib/ipc";
import { useEngineStore } from "@/stores/engine";
import { useEntitlementStore } from "@/stores/entitlement";
import { useAccountStore } from "@/stores/account";

function arrange(opts: { entitled: boolean; scope?: "all" | "only" }) {
  const base = useEngineStore.getState().state;
  useEngineStore.setState({
    state: {
      ...base,
      surround3d: { ...base.surround3d, enabled: true },
      room: { ...base.room, enabled: true },
      convolver: { ...base.convolver, enabled: true, irId: "/ir.wav", irName: "ir" },
      systemEqScope: { mode: opts.scope ?? "all", apps: opts.scope === "only" ? ["a.app"] : [] },
    },
  });
  useEntitlementStore.setState({ premium: opts.entitled, supporter: false, loading: false });
  useAccountStore.setState({ status: null });
}

const st = () => useEngineStore.getState().state;

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
});

describe("enforcePlus() for an account that is not entitled", () => {
  it("turns saved Plus effects off but keeps their settings, and remembers them", () => {
    arrange({ entitled: false });
    enforcePlus();

    expect(st().surround3d.enabled).toBe(false);
    expect(st().room.enabled).toBe(false);
    expect(st().convolver.enabled).toBe(false);
    // Settings (e.g. the loaded IR) are untouched.
    expect(st().convolver.irId).toBe("/ir.wav");
    expect(engineSetSurround3d).toHaveBeenCalledWith(false, expect.anything(), expect.anything(), expect.anything());
    expect(engineSetRoom).toHaveBeenCalled();
    expect(engineSetConvolver).toHaveBeenCalled();
    expect(readSuppressed()).toMatchObject({ surround3d: true, room: true, convolver: true });
  });

  it("falls back to all-apps for a per-app EQ scope and remembers the old one", () => {
    arrange({ entitled: false, scope: "only" });
    enforcePlus();

    expect(applySystemEqScope).toHaveBeenCalledWith({ mode: "all", apps: [] });
    expect(readSuppressed().scope).toEqual({ mode: "only", apps: ["a.app"] });
  });

  it("does nothing when no Plus effect is active", () => {
    arrange({ entitled: false });
    useEngineStore.setState({
      state: {
        ...st(),
        surround3d: { ...st().surround3d, enabled: false },
        room: { ...st().room, enabled: false },
        convolver: { ...st().convolver, enabled: false },
      },
    });
    enforcePlus();
    expect(engineSetRoom).not.toHaveBeenCalled();
    expect(readSuppressed()).toEqual({});
  });
});

describe("enforcePlus() for an entitled account", () => {
  it("never touches active effects (a paying user, even offline from cache)", () => {
    arrange({ entitled: true, scope: "only" });
    enforcePlus();

    expect(st().surround3d.enabled).toBe(true);
    expect(st().room.enabled).toBe(true);
    expect(st().convolver.enabled).toBe(true);
    expect(applySystemEqScope).not.toHaveBeenCalled();
    expect(readSuppressed()).toEqual({});
  });

  it("brings suppressed effects and the old scope back once entitlement arrives", () => {
    arrange({ entitled: false, scope: "only" });
    enforcePlus();
    useEngineStore.setState({
      state: { ...st(), systemEqScope: { mode: "all", apps: [] } },
    });
    vi.mocked(applySystemEqScope).mockClear();

    // They buy Plus (or a late entitlement fetch lands).
    useEntitlementStore.setState({ supporter: true });
    enforcePlus();

    expect(st().surround3d.enabled).toBe(true);
    expect(st().room.enabled).toBe(true);
    expect(st().convolver.enabled).toBe(true);
    expect(applySystemEqScope).toHaveBeenCalledWith({ mode: "only", apps: ["a.app"] });
    expect(readSuppressed()).toEqual({});
  });

  it("is idempotent", () => {
    arrange({ entitled: false });
    enforcePlus();
    vi.clearAllMocks();
    enforcePlus();
    expect(engineSetRoom).not.toHaveBeenCalled();
  });
});
