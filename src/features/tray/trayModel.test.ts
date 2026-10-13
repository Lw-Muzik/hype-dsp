import { describe, expect, it } from "vitest";
import { useEngineStore } from "@/stores/engine";
import { buildTrayModel, sameTrayModel, type TrayInputs } from "./trayModel";

const base = (o: Partial<TrayInputs> = {}): TrayInputs => ({
  nowPlaying: null,
  playing: false,
  paused: false,
  queueLength: 0,
  queueIndex: -1,
  mode: "player",
  modeSupported: true,
  engineState: useEngineStore.getState().state,
  presets: [],
  visualizerAvailable: true,
  visualizerRunning: false,
  updateStatus: { state: "idle" },
  ...o,
});

describe("buildTrayModel", () => {
  it("playing requires playing && !paused", () => {
    expect(buildTrayModel(base({ playing: true, paused: false })).playing).toBe(true);
    expect(buildTrayModel(base({ playing: true, paused: true })).playing).toBe(false);
    expect(buildTrayModel(base({ playing: false })).playing).toBe(false);
  });

  it("canControl needs a loaded track or a queued one", () => {
    expect(buildTrayModel(base()).canControl).toBe(false);
    expect(buildTrayModel(base({ nowPlaying: "Song" })).canControl).toBe(true);
    expect(buildTrayModel(base({ queueLength: 3, queueIndex: 1 })).canControl).toBe(true);
    expect(buildTrayModel(base({ queueLength: 3, queueIndex: -1 })).canControl).toBe(false);
  });

  it("routes follow the mode", () => {
    const p = buildTrayModel(base({ mode: "player" })).routes.map((r) => r.id);
    const e = buildTrayModel(base({ mode: "equalizer" })).routes.map((r) => r.id);
    expect(p).toContain("mixer");
    expect(e).toEqual(["equalizer", "enhancer", "stems", "visuals"]);
    expect(e).not.toContain("mixer");
    expect(buildTrayModel(base({ mode: "equalizer" })).mode).toBe("equalizer");
  });

  it("effects exclude presets and trim the ' on' suffix", () => {
    const fx = buildTrayModel(base()).effects;
    expect(fx.find((f) => f.id === "presets")).toBeUndefined();
    expect(fx.find((f) => f.id === "bass")?.label).toBe("Bass boost");
    expect(fx.find((f) => f.id === "headphone")?.label).toBe("Headphone correction");
    expect(fx.find((f) => f.id === "presets")).toBeUndefined();
  });

  it("reflects on/off and the headphone rule", () => {
    const s = useEngineStore.getState().state;
    const off = buildTrayModel(
      base({ engineState: { ...s, bass: { ...s.bass, enabled: false }, headphone: { ...s.headphone, enabled: false } } }),
    ).effects;
    expect(off.find((f) => f.id === "bass")).toMatchObject({ on: false, enabled: true });
    expect(off.find((f) => f.id === "headphone")).toMatchObject({ on: false, enabled: false });
    const on = buildTrayModel(
      base({ engineState: { ...s, bass: { ...s.bass, enabled: true }, headphone: { ...s.headphone, enabled: true } } }),
    ).effects;
    expect(on.find((f) => f.id === "bass")?.on).toBe(true);
    expect(on.find((f) => f.id === "headphone")).toMatchObject({ on: true, enabled: true });
  });

  it("marks the active preset and passes power through", () => {
    const s = useEngineStore.getState().state;
    const m = buildTrayModel(
      base({
        engineState: { ...s, power: false, activePresetId: "b" },
        presets: [{ id: "a", name: "A" }, { id: "b", name: "B" }],
      }),
    );
    expect(m.power).toBe(false);
    expect(m.presets).toEqual([
      { id: "a", name: "A", active: false },
      { id: "b", name: "B", active: true },
    ]);
  });

  it("visualizer is null when unavailable, else running", () => {
    expect(buildTrayModel(base({ visualizerAvailable: false, visualizerRunning: true })).visualizer).toBeNull();
    expect(buildTrayModel(base({ visualizerRunning: true })).visualizer).toBe(true);
    expect(buildTrayModel(base({ visualizerRunning: false })).visualizer).toBe(false);
  });

  it("updateVersion only when ready", () => {
    expect(buildTrayModel(base({ updateStatus: { state: "ready", version: "1.2.3", notes: null } })).updateVersion).toBe("1.2.3");
    expect(buildTrayModel(base({ updateStatus: { state: "downloading", version: "1.2.3", received: 1, total: 2 } })).updateVersion).toBeNull();
    expect(buildTrayModel(base()).updateVersion).toBeNull();
  });
});

describe("sameTrayModel", () => {
  it("compares deeply", () => {
    expect(sameTrayModel(buildTrayModel(base()), buildTrayModel(base()))).toBe(true);
    expect(sameTrayModel(buildTrayModel(base()), buildTrayModel(base({ playing: true })))).toBe(false);
    expect(sameTrayModel(null, buildTrayModel(base()))).toBe(false);
    expect(sameTrayModel(null, null)).toBe(true);
  });
});
