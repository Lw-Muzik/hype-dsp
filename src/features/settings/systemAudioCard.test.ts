import { describe, expect, it } from "vitest";

import type { SystemAudioStatus } from "../../lib/ipc";
import { systemAudioAffordance } from "./systemAudioCard";

const status = (over: Partial<SystemAudioStatus>): SystemAudioStatus => ({
  supported: true,
  available: false,
  driverInstalled: false,
  needsDriver: false,
  driverBundled: true,
  apoInstalled: false,
  ...over,
});

describe("systemAudioAffordance", () => {
  it("offers Enable when the pipeline is ready", () => {
    // macOS tap present / Linux Pulse present / Windows driver installed.
    expect(
      systemAudioAffordance(status({ available: true, driverInstalled: true })),
    ).toBe("enable");
  });

  it("offers Install when the bundled driver is present but not installed", () => {
    expect(
      systemAudioAffordance(status({ needsDriver: true, driverBundled: true })),
    ).toBe("install");
  });

  it("reports not-bundled for a Windows build that ships no driver package", () => {
    // The v0.1.12 field bug: the card offered "Install audio driver" even though
    // the build had no .inf to install — the click could only dead-end in a
    // developer-facing error. A driverless build must say so instead.
    expect(systemAudioAffordance(status({ driverBundled: false }))).toBe(
      "not-bundled",
    );
  });

  it("reports unavailable when the OS needs no driver but the backend is absent", () => {
    // e.g. Linux without PulseAudio/PipeWire, macOS tap unavailable.
    expect(systemAudioAffordance(status({}))).toBe("unavailable");
  });

  it("prefers Enable over everything once available, even mid-poll", () => {
    // After a successful in-app install the device enumerates while the old
    // status flags are still being re-fetched.
    expect(
      systemAudioAffordance(
        status({ available: true, needsDriver: true, driverBundled: true }),
      ),
    ).toBe("enable");
  });
});
