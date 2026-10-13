// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return { ...actual };
});
vi.mock("@/components/PremiumGate", () => ({ requirePremium: vi.fn(() => true) }));

import { requirePremium } from "@/components/PremiumGate";
import { useEngineStore } from "@/stores/engine";
import { useUiStore } from "@/stores/ui";
import { EFFECTS, useEffectToggle } from "./useEffectToggles";
import type { EffectId } from "./useEffectToggles";
import { EffectsRack } from "./EffectsRack";
import { RoomCard } from "../RoomCard";

const gate = vi.mocked(requirePremium);
const spies = {
  setEqEnabled: vi.fn(),
  setBass: vi.fn(),
  setSpatializer: vi.fn(),
  setSurround3d: vi.fn(),
  setRoom: vi.fn(),
  setConvolver: vi.fn(),
  setCompander: vi.fn(),
  setSaturation: vi.fn(),
  setScriptEnabled: vi.fn(),
  clearProfile: vi.fn(),
  setOutput: vi.fn(),
};

beforeEach(() => {
  gate.mockReset();
  gate.mockReturnValue(true);
  Object.values(spies).forEach((s) => s.mockReset());
  useEngineStore.setState(spies);
});
afterEach(cleanup);

const st = () => useEngineStore.getState().state;
const toggle = (id: EffectId) => renderHook(() => useEffectToggle(id)).result.current;

describe("useEffectToggle calls", () => {
  it("equalizer", () => {
    toggle("equalizer").toggle(false);
    expect(spies.setEqEnabled).toHaveBeenCalledWith(false);
  });
  it("bass", () => {
    const b = st().bass;
    toggle("bass").toggle(true);
    expect(spies.setBass).toHaveBeenCalledWith(true, b.amount, b.harmonics, b.adaptive);
  });
  it("surround", () => {
    const s = st().spatializer;
    toggle("surround").toggle(true);
    expect(spies.setSpatializer).toHaveBeenCalledWith(true, s.amount, s.mode);
  });
  it.each([
    ["surround3d", "setSurround3d", "surround3d", "SURROUND_3D"],
    ["room", "setRoom", "room", "ROOM_REVERB"],
    ["convolver", "setConvolver", "convolver", "CONVOLVER"],
  ] as const)("%s sets enabled and gates only turning on", (id, setter, key, feature) => {
    const t = toggle(id);
    t.toggle(true);
    expect(gate).toHaveBeenCalledWith(feature);
    expect(spies[setter]).toHaveBeenLastCalledWith({ ...st()[key], enabled: true });

    spies[setter].mockClear();
    gate.mockReset();
    gate.mockReturnValue(false);
    t.toggle(true);
    expect(spies[setter]).not.toHaveBeenCalled();

    t.toggle(false);
    expect(spies[setter]).toHaveBeenCalledWith({ ...st()[key], enabled: false });
  });
  it("compander", () => {
    toggle("compander").toggle(true);
    expect(spies.setCompander).toHaveBeenCalledWith({ ...st().compander, enabled: true });
  });
  it("saturation", () => {
    toggle("saturation").toggle(true);
    expect(spies.setSaturation).toHaveBeenCalledWith({ ...st().saturation, enabled: true });
  });
  it("script", () => {
    toggle("script").toggle(true);
    expect(spies.setScriptEnabled).toHaveBeenCalledWith(true);
  });
  it("limiter", () => {
    toggle("limiter").toggle(false);
    expect(spies.setOutput).toHaveBeenCalledWith({ ...st().output, limiterEnabled: false });
  });
  it("headphone cannot turn on, turns off via clearProfile", () => {
    useEngineStore.setState({
      state: { ...st(), headphone: { enabled: false, preamp: 0, bands: [] } },
    });
    const off = toggle("headphone");
    expect(off.canToggleOn).toBe(false);
    off.toggle(true);
    expect(spies.clearProfile).not.toHaveBeenCalled();
    off.toggle(false);
    expect(spies.clearProfile).toHaveBeenCalledTimes(1);
  });
});

describe("EffectsRack", () => {
  it("renders 12 tiles in order with on/off and the count", () => {
    const s = st();
    useEngineStore.setState({
      state: {
        ...s,
        eq: { ...s.eq, enabled: true },
        bass: { ...s.bass, enabled: true },
        room: { ...s.room, enabled: false },
        output: { ...s.output, limiterEnabled: false },
        headphone: { enabled: false, preamp: 0, bands: [] },
      },
    });
    const { container } = render(<EffectsRack />);
    const tiles = [...container.querySelectorAll("li[data-effect]")];
    expect(tiles.map((t) => t.getAttribute("data-effect"))).toEqual(EFFECTS.map((e) => e.id));
    expect(tiles).toHaveLength(12);
    expect(screen.getByRole("switch", { name: "Bass boost on" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Room reverb on" })).not.toBeChecked();
    expect(screen.getByRole("switch", { name: "Headphone correction on" })).toBeDisabled();
    expect(screen.queryByRole("switch", { name: "Presets" })).toBeNull();
    const n = tiles.filter((t) => t.getAttribute("data-on") === "true").length;
    expect(screen.getByText(`${n} on`)).toBeInTheDocument();
  });

  it("switch drives the shared toggle", () => {
    render(<EffectsRack />);
    fireEvent.click(screen.getByRole("switch", { name: "Tube saturation on" }));
    expect(spies.setSaturation).toHaveBeenCalled();
  });

  it("clicking a tile scrolls to the card", () => {
    const el = document.createElement("div");
    el.id = "fx-bass";
    el.scrollIntoView = vi.fn();
    document.body.appendChild(el);
    render(<EffectsRack />);
    fireEvent.click(screen.getByRole("button", { name: "Bass" }));
    expect(el.scrollIntoView).toHaveBeenCalled();
    expect(el.classList.contains("ring-2")).toBe(true);
    el.remove();
  });

  it("Equalizer tile navigates", () => {
    const setRoute = vi.fn();
    useUiStore.setState({ setRoute });
    render(<EffectsRack />);
    fireEvent.click(screen.getByRole("button", { name: "Equalizer" }));
    expect(setRoute).toHaveBeenCalledWith("equalizer");
  });
});

describe("RoomCard uses the shared hook", () => {
  it("flips through the gate and setter", () => {
    render(<RoomCard />);
    act(() => {
      fireEvent.click(screen.getByRole("switch", { name: "Enable room reverb" }));
    });
    expect(gate).toHaveBeenCalledWith("ROOM_REVERB");
    expect(spies.setRoom).toHaveBeenCalledWith({ ...st().room, enabled: !st().room.enabled });
  });
});
