import { describe, expect, it } from "vitest";
import { DRIFT_TOLERANCE_SECS, syncAction } from "@/features/player/videoSync";

const inputs = (over: Partial<Parameters<typeof syncAction>[0]> = {}) => ({
  enginePos: 10,
  videoPos: 10,
  paused: false,
  videoPaused: false,
  ready: true,
  ...over,
});

describe("syncAction", () => {
  it("leaves the picture alone while it's close enough to see as matching", () => {
    const a = syncAction(inputs({ videoPos: 10 + DRIFT_TOLERANCE_SECS / 2 }));
    expect(a.seekTo).toBeNull();
    expect(a.setPaused).toBeNull();
  });

  /** A seek costs a decode flush and a visible stutter, so it must buy more than
   *  it spends: drift under the tolerance is invisible, a correction isn't. */
  it("does not correct drift exactly at the tolerance", () => {
    expect(syncAction(inputs({ videoPos: 10 + DRIFT_TOLERANCE_SECS })).seekTo).toBeNull();
  });

  it("pulls the picture to the engine once the drift is visible", () => {
    expect(syncAction(inputs({ enginePos: 10, videoPos: 12 })).seekTo).toBe(10);
    expect(syncAction(inputs({ enginePos: 10, videoPos: 8 })).seekTo).toBe(10);
  });

  /** The engine is the truth in both directions — a video that has stalled and a
   *  video that has run ahead are the same problem. */
  it("corrects a lost video the same way as a drifting one", () => {
    expect(syncAction(inputs({ enginePos: 100, videoPos: 3 })).seekTo).toBe(100);
  });

  it("follows the engine's transport", () => {
    expect(syncAction(inputs({ paused: true, videoPaused: false })).setPaused).toBe(true);
    expect(syncAction(inputs({ paused: false, videoPaused: true })).setPaused).toBe(false);
    expect(syncAction(inputs({ paused: true, videoPaused: true })).setPaused).toBeNull();
  });

  /** Seeking an element that hasn't buffered fights its loader, and its clock
   *  means nothing yet. Transport still applies: a video left playing while the
   *  engine is paused runs away, and the drift grows without bound. */
  it("waits for data before correcting position, but still follows transport", () => {
    const a = syncAction(inputs({ ready: false, videoPos: 0, enginePos: 30, paused: true }));
    expect(a.seekTo).toBeNull();
    expect(a.setPaused).toBe(true);
  });

  /** The whole design in one assertion: nothing this returns can touch audio. */
  it("only ever describes what the picture should do", () => {
    const a = syncAction(inputs({ enginePos: 10, videoPos: 99, paused: true }));
    expect(Object.keys(a).sort()).toEqual(["seekTo", "setPaused"]);
  });
});
