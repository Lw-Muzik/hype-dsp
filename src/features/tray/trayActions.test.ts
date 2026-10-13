import { describe, expect, it, vi } from "vitest";
import { runTrayAction, type TrayActionDeps } from "./trayActions";
import { toggleEffect } from "@/features/enhancer/effects/useEffectToggles";
import { useEngineStore } from "@/stores/engine";

vi.mock("@/lib/ipc", async (orig) => ({
  ...(await orig<typeof import("@/lib/ipc")>()),
  eqApplyPreset: vi.fn(),
}));
vi.mock("@/components/PremiumGate", () => ({ requirePremium: vi.fn(() => false) }));

function deps(o: Partial<TrayActionDeps> = {}): TrayActionDeps {
  return {
    mediaCommand: vi.fn(),
    switchMode: vi.fn(async () => "switched"),
    getMode: vi.fn(() => "player" as const),
    getPower: vi.fn(() => true),
    setPower: vi.fn(),
    isEffectOn: vi.fn(() => false),
    toggleEffect: vi.fn(),
    applyPreset: vi.fn(async () => {}),
    setRoute: vi.fn(),
    isRouteVisible: vi.fn((r, m) =>
      m === "equalizer" ? ["equalizer", "enhancer", "stems", "visuals"].includes(r) : true,
    ),
    toggleVisualizer: vi.fn(),
    restartUpdate: vi.fn(async () => {}),
    ...o,
  };
}

describe("runTrayAction", () => {
  it("transport", async () => {
    const d = deps();
    await runTrayAction("play_pause", d);
    await runTrayAction("next", d);
    await runTrayAction("prev", d);
    expect(vi.mocked(d.mediaCommand).mock.calls).toEqual([["toggle"], ["next"], ["prev"]]);
  });

  it("mode switches only when the target differs from the current mode", async () => {
    const fromPlayer = deps({ getMode: vi.fn(() => "player" as const) });
    await runTrayAction("mode:equalizer", fromPlayer);
    await runTrayAction("mode:player", fromPlayer); // already Player: no-op
    expect(vi.mocked(fromPlayer.switchMode).mock.calls).toEqual([["equalizer"]]);

    const fromEq = deps({ getMode: vi.fn(() => "equalizer" as const) });
    await runTrayAction("mode:equalizer", fromEq); // already Equalizer: no tap rebuild
    await runTrayAction("mode:player", fromEq);
    expect(vi.mocked(fromEq.switchMode).mock.calls).toEqual([["player"]]);
  });

  it("power flips the current value", async () => {
    const d = deps({ getPower: vi.fn(() => true) });
    await runTrayAction("power", d);
    expect(d.setPower).toHaveBeenCalledWith(false);
    const d2 = deps({ getPower: vi.fn(() => false) });
    await runTrayAction("power", d2);
    expect(d2.setPower).toHaveBeenCalledWith(true);
  });

  it("fx toggles to the opposite of the current state", async () => {
    const d = deps({ isEffectOn: vi.fn(() => false) });
    await runTrayAction("fx:bass", d);
    expect(d.toggleEffect).toHaveBeenCalledWith("bass", true);
    const d2 = deps({ isEffectOn: vi.fn(() => true) });
    await runTrayAction("fx:room", d2);
    expect(d2.toggleEffect).toHaveBeenCalledWith("room", false);
  });

  it("ignores fx:presets and unknown effects", async () => {
    const d = deps();
    await runTrayAction("fx:presets", d);
    await runTrayAction("fx:nope", d);
    expect(d.toggleEffect).not.toHaveBeenCalled();
  });

  it("preset, presets:open, visualizer, update", async () => {
    const d = deps();
    await runTrayAction("preset:rock", d);
    expect(d.applyPreset).toHaveBeenCalledWith("rock");
    await runTrayAction("presets:open", d);
    expect(d.setRoute).toHaveBeenCalledWith("equalizer");
    await runTrayAction("visualizer", d);
    expect(d.toggleVisualizer).toHaveBeenCalled();
    await runTrayAction("update_restart", d);
    expect(d.restartUpdate).toHaveBeenCalled();
  });

  it("go:<route> navigates only to visible routes", async () => {
    const d = deps();
    await runTrayAction("go:mixer", d);
    expect(d.setRoute).toHaveBeenCalledWith("mixer");
    const e = deps({ getMode: vi.fn(() => "equalizer" as const) });
    await runTrayAction("go:mixer", e);
    expect(e.setRoute).not.toHaveBeenCalled();
    await runTrayAction("go:enhancer", e);
    expect(e.setRoute).toHaveBeenCalledWith("enhancer");
  });

  it("ignores unknown ids and swallows errors", async () => {
    const d = deps({ mediaCommand: vi.fn(() => { throw new Error("x"); }) });
    await runTrayAction("bogus", d);
    await expect(runTrayAction("next", d)).resolves.toBeUndefined();
    expect(d.setRoute).not.toHaveBeenCalled();
    expect(d.setPower).not.toHaveBeenCalled();
  });
});

describe("hook semantics through the real toggleEffect", () => {
  it("fx:headphone on is a no-op (needs a profile)", () => {
    const s = useEngineStore.getState();
    const clear = vi.spyOn(s, "clearProfile").mockImplementation(() => {});
    toggleEffect("headphone", true);
    expect(clear).not.toHaveBeenCalled();
    toggleEffect("headphone", false);
    expect(clear).toHaveBeenCalled();
  });

  it("premium-gated effects stay off when the gate refuses", () => {
    const setRoom = vi.spyOn(useEngineStore.getState(), "setRoom").mockImplementation(() => {});
    toggleEffect("room", true);
    expect(setRoom).not.toHaveBeenCalled();
    toggleEffect("room", false);
    expect(setRoom).toHaveBeenCalled();
  });
});
