import { describe, expect, it } from "vitest";
import { linkedMessage } from "./ConnectCard";
import type { DeviceInfo } from "./protocol";

const d = (id: string, kind: DeviceInfo["kind"], name: string, seen: string): DeviceInfo => ({
  id, kind, name, platform: null, appVersion: null, caps: ["player"], online: true, lastSeenAt: seen,
});

describe("linkedMessage", () => {
  it("names the most recently seen phone", () => {
    const msg = linkedMessage([
      d("a", "mobile", "Old Phone", "2026-01-01T00:00:00Z"),
      d("b", "mobile", "Pixel 8", "2026-10-13T10:00:00Z"),
      d("c", "desktop", "MacBook", "2026-12-01T00:00:00Z"),
    ]);
    expect(msg).toContain("Pixel 8");
    expect(msg).not.toContain("Old Phone");
    expect(msg).not.toContain("MacBook");
  });
  it("stays generic when no phone is known", () => {
    expect(linkedMessage([d("c", "desktop", "MacBook", "x")])).toBe("Linked — this desktop now shares your phone's account.");
  });
});
