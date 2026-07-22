import { describe, expect, it } from "vitest";
import { dedupeRadioTracks, radioStep, RADIO_LOW_WATER } from "@/stores/radio";
import { ytmusicItem } from "@/stores/engine";
import type { YtTrack } from "@/lib/types";

const track = (videoId: string, over: Partial<YtTrack> = {}): YtTrack => ({
  videoId,
  title: `Song ${videoId}`,
  artist: "Artist",
  album: null,
  durationSecs: 200,
  thumbnail: null,
  playlistId: "RDAMVMseed",
  playlistTitle: "Radio",
  isAvailable: true,
  hasVideo: false,
  ...over,
});

/** A store snapshot near the end of a 10-track all-YT queue. */
const base = () => ({
  autoplay: true,
  fetching: false,
  session: { seedId: "seed", continuation: "tok" } as {
    seedId: string;
    continuation: string | null;
  } | null,
  orderLen: 10,
  orderPos: 10 - RADIO_LOW_WATER - 1, // exactly LOW_WATER tracks remain ahead
  allYtMusic: true,
  lastVideoId: "last",
  repeat: "off" as const,
});

describe("radioStep", () => {
  it("continues the session with its token when the queue runs low", () => {
    expect(radioStep(base())).toEqual({ kind: "continue", seedId: "seed", token: "tok" });
  });

  it("does nothing while plenty of queue remains", () => {
    expect(radioStep({ ...base(), orderPos: 0 })).toBeNull();
  });

  it("waits until exactly LOW_WATER remain — one more track ahead is not yet low", () => {
    // remaining = 10 - 3 - 1 = RADIO_LOW_WATER + 1 → not yet.
    expect(radioStep({ ...base(), orderPos: 3 })).toBeNull();
  });

  it("is fully gated by the Autoplay switch", () => {
    expect(radioStep({ ...base(), autoplay: false })).toBeNull();
  });

  it("never stacks fetches", () => {
    expect(radioStep({ ...base(), fetching: true })).toBeNull();
  });

  it("lets repeat win — a loop over a growing queue would never come round", () => {
    expect(radioStep({ ...base(), repeat: "all" })).toBeNull();
    expect(radioStep({ ...base(), repeat: "one" })).toBeNull();
  });

  it("re-seeds from the last track when the chain has no token", () => {
    expect(radioStep({ ...base(), session: { seedId: "seed", continuation: null } })).toEqual({
      kind: "reseed",
      seedId: "last",
    });
  });

  it("starts a session for a finishing all-YT queue with none", () => {
    expect(radioStep({ ...base(), session: null })).toEqual({ kind: "start", seedId: "last" });
  });

  it("never grows a local/phone/cloud queue", () => {
    expect(radioStep({ ...base(), session: null, allYtMusic: false })).toBeNull();
  });

  it("does nothing on an empty or unstarted queue", () => {
    expect(radioStep({ ...base(), orderLen: 0, orderPos: -1 })).toBeNull();
  });
});

describe("dedupeRadioTracks", () => {
  const queue = [ytmusicItem(track("a")), ytmusicItem(track("b"))];

  it("drops tracks already queued — continuation pages overlap", () => {
    const out = dedupeRadioTracks(queue, [track("b"), track("c")]);
    expect(out.map((t) => t.videoId)).toEqual(["c"]);
  });

  it("drops in-batch duplicates but keeps the order", () => {
    const out = dedupeRadioTracks(queue, [track("d"), track("c"), track("d")]);
    expect(out.map((t) => t.videoId)).toEqual(["d", "c"]);
  });

  it("drops unavailable tracks — they can't stream", () => {
    const out = dedupeRadioTracks(queue, [track("e", { isAvailable: false })]);
    expect(out).toEqual([]);
  });
});
