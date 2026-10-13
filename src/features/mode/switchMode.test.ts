import { describe, expect, it, vi } from "vitest";
import type { SystemAudioStatus } from "@/lib/ipc";
import { switchMode, type SwitchDeps } from "./switchMode";

const status = (o: Partial<SystemAudioStatus> = {}): SystemAudioStatus =>
  ({
    supported: true,
    available: true,
    driverInstalled: true,
    needsDriver: false,
    driverBundled: true,
    ...o,
  }) as SystemAudioStatus;

function makeDeps(o: Partial<SwitchDeps> = {}) {
  let enabled = false;
  const deps: SwitchDeps = {
    requirePremium: vi.fn(() => true),
    systemAudioStatus: vi.fn(async () => status()),
    openSettings: vi.fn(async () => {}),
    enable: vi.fn(async () => {
      enabled = true;
    }),
    disable: vi.fn(async () => {
      enabled = false;
    }),
    eqState: () => ({ enabled, error: null }),
    setRoute: vi.fn(),
    toast: { error: vi.fn(), info: vi.fn() },
    ...o,
  };
  return deps;
}

describe("switchMode", () => {
  it("is blocked by the premium gate", async () => {
    const d = makeDeps({ requirePremium: vi.fn(() => false) });
    expect(await switchMode("equalizer", d)).toBe("blocked");
    expect(d.enable).not.toHaveBeenCalled();
  });

  it("reports unsupported machines", async () => {
    const d = makeDeps({ systemAudioStatus: vi.fn(async () => status({ supported: false })) });
    expect(await switchMode("equalizer", d)).toBe("unsupported");
    expect(d.toast.error).toHaveBeenCalledWith("Equalizer mode isn't available on this computer.");
  });

  it("opens Settings at system-audio when setup is needed", async () => {
    const d = makeDeps({
      systemAudioStatus: vi.fn(async () =>
        status({ available: false, needsDriver: true, driverBundled: true }),
      ),
    });
    expect(await switchMode("equalizer", d)).toBe("needs-setup");
    expect(d.openSettings).toHaveBeenCalledWith("system-audio");
    expect(d.toast.info).toHaveBeenCalled();
    expect(d.enable).not.toHaveBeenCalled();
  });

  it("toasts the store error when enabling fails", async () => {
    const d = makeDeps({
      enable: vi.fn(async () => {}),
      eqState: () => ({ enabled: false, error: "no permission" }),
    });
    expect(await switchMode("equalizer", d)).toBe("failed");
    expect(d.toast.error).toHaveBeenCalledWith("Couldn't switch to Equalizer mode: no permission");
    expect(d.setRoute).not.toHaveBeenCalled();
  });

  it("enables, then lands on the equalizer route", async () => {
    const d = makeDeps();
    expect(await switchMode("equalizer", d)).toBe("switched");
    expect(d.setRoute).toHaveBeenCalledWith("equalizer");
  });

  it("tries enabling anyway when the status check errors", async () => {
    const d = makeDeps({ systemAudioStatus: vi.fn(async () => Promise.reject(new Error("x"))) });
    expect(await switchMode("equalizer", d)).toBe("switched");
  });

  it("player disables and keeps the route", async () => {
    const d = makeDeps();
    expect(await switchMode("player", d)).toBe("switched");
    expect(d.disable).toHaveBeenCalled();
    expect(d.setRoute).not.toHaveBeenCalled();
  });

  it("returns busy for a concurrent call", async () => {
    let release!: () => void;
    const d = makeDeps({
      systemAudioStatus: vi.fn(() => new Promise<SystemAudioStatus>((r) => (release = () => r(status())))),
    });
    const first = switchMode("equalizer", d);
    expect(await switchMode("player", d)).toBe("busy");
    expect(d.disable).not.toHaveBeenCalled();
    release();
    expect(await first).toBe("switched");
  });
});
