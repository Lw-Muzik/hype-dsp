import { describe, expect, it } from "vitest";

import type { SetupProgressEvent } from "./setupProgress";
import { setupPercent, setupStatusLine } from "./setupProgress";

const ev = (over: Partial<SetupProgressEvent>): SetupProgressEvent => ({
  tool: "yt-dlp",
  phase: "downloading",
  received: 0,
  total: null,
  ...over,
});

describe("setupStatusLine", () => {
  it("shows received of total while downloading", () => {
    const e = ev({ received: 12 * 1024 * 1024, total: 17 * 1024 * 1024 });
    expect(setupStatusLine(e)).toBe("Downloading yt-dlp… 12 of 17 MB");
  });

  it("drops the total when the server did not send one", () => {
    const e = ev({ received: 5 * 1024 * 1024 });
    expect(setupStatusLine(e)).toBe("Downloading yt-dlp… 5 MB");
  });

  it("names the tool in every phase", () => {
    expect(setupStatusLine(ev({ tool: "ffmpeg", phase: "verifying" }))).toBe(
      "Verifying ffmpeg…",
    );
    expect(setupStatusLine(ev({ tool: "ffmpeg", phase: "installing" }))).toBe(
      "Installing ffmpeg…",
    );
  });

  it("has a fallback for phases it does not know", () => {
    expect(setupStatusLine(ev({ phase: "later-addition" }))).toBe(
      "Setting up yt-dlp…",
    );
  });
});

describe("setupPercent", () => {
  it("is a clamped percentage while downloading with a known total", () => {
    expect(setupPercent(ev({ received: 50, total: 200 }))).toBe(25);
    // Servers occasionally under-report totals; the bar must not overflow.
    expect(setupPercent(ev({ received: 300, total: 200 }))).toBe(100);
  });

  it("is indeterminate without a total or outside downloading", () => {
    expect(setupPercent(ev({ received: 5 }))).toBeNull();
    expect(setupPercent(ev({ phase: "verifying", received: 1, total: 2 }))).toBeNull();
  });
});
