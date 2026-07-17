import { describe, it, expect } from "vitest";
import {
  observe,
  effectiveMode,
  chooseStreamMode,
  UNKNOWN_NETWORK,
  type NetworkState,
} from "./networkMode";

const T0 = 1_700_000_000_000;
const MIN = 60_000;

/** Throughput as it is actually reported for a streamed track: the download is
 *  backpressured to playback rate, so the figure is roughly the track's bitrate
 *  — ~30 KB/s for YT Music m4a, not the megabytes/sec a link can really do. */
const REAL_MEASURED_BPS = 30_000;

const clean = { downloadBps: REAL_MEASURED_BPS, rebufferDelta: 0 };
const rebuffered = { downloadBps: REAL_MEASURED_BPS, rebufferDelta: 1 };

describe("observe", () => {
  it("does not constrain a link over a single rebuffer", () => {
    const s = observe(UNKNOWN_NETWORK, rebuffered, T0);
    expect(s.mode).toBe("unknown");
  });

  it("constrains only once rebuffers are sustained", () => {
    let s: NetworkState = UNKNOWN_NETWORK;
    s = observe(s, rebuffered, T0);
    s = observe(s, rebuffered, T0);
    expect(s.mode).toBe("unknown");
    s = observe(s, rebuffered, T0);
    expect(s.mode).toBe("constrained");
    expect(s.at).toBe(T0);
  });

  it("counts a multi-rebuffer sample by how many it reports", () => {
    const s = observe(UNKNOWN_NETWORK, { downloadBps: 0, rebufferDelta: 3 }, T0);
    expect(s.mode).toBe("constrained");
  });

  it("promotes a link that genuinely measures fast", () => {
    const s = observe(UNKNOWN_NETWORK, { downloadBps: 500_000, rebufferDelta: 0 }, T0);
    expect(s.mode).toBe("fast");
  });
});

describe("effectiveMode", () => {
  it("lets a constrained verdict expire so the link is re-tried", () => {
    const s = observe(observe(observe(UNKNOWN_NETWORK, rebuffered, T0), rebuffered, T0), rebuffered, T0);
    expect(s.mode).toBe("constrained");
    expect(effectiveMode(s, T0 + MIN)).toBe("constrained");
    expect(effectiveMode(s, T0 + 11 * MIN)).toBe("unknown");
  });

  it("never expires a fast verdict", () => {
    const s = observe(UNKNOWN_NETWORK, { downloadBps: 500_000, rebufferDelta: 0 }, T0);
    expect(effectiveMode(s, T0 + 24 * 60 * MIN)).toBe("fast");
  });
});

describe("chooseStreamMode", () => {
  it("is optimistic about an unmeasured link", () => {
    expect(chooseStreamMode("ytmusic", false, "unknown")).toBe("gapless");
  });

  it("honours Data Saver", () => {
    expect(chooseStreamMode("ytmusic", true, "fast")).toBe("progressive");
  });

  it("drops a constrained link to the single-track path", () => {
    expect(chooseStreamMode("ytmusic", false, "constrained")).toBe("progressive");
  });
});

/** The reported bug: "the crossfade completely doesn't work".
 *
 *  Crossfade needs the gapless path, and a "constrained" verdict routes to the
 *  single-track source, which has no next track to fade to. The old classifier
 *  demoted on the first rebuffer and could only clear itself by observing
 *  400 KB/s — a figure a backpressured download never reports. One hiccup and
 *  crossfade was gone for the session, with no way back. */
describe("a constrained link recovers", () => {
  it("does not strand crossfade forever after a rebuffer, however long it plays cleanly", () => {
    let s: NetworkState = UNKNOWN_NETWORK;
    for (let i = 0; i < 3; i++) s = observe(s, rebuffered, T0);
    expect(chooseStreamMode("ytmusic", false, effectiveMode(s, T0))).toBe("progressive");

    // An hour of clean playback. Every sample reports the true measured rate,
    // which is the track's bitrate and never reaches the "fast" threshold — so
    // evidence alone can never clear the verdict.
    let now = T0;
    for (let i = 0; i < 60; i++) {
      now += MIN;
      s = observe(s, clean, now);
    }
    expect(s.mode).toBe("constrained");

    // Time is what clears it. Crossfade comes back.
    expect(chooseStreamMode("ytmusic", false, effectiveMode(s, now))).toBe("gapless");
  });
});
