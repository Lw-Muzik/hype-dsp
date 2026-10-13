import { afterEach, describe, expect, it, vi } from "vitest";
import type { LyricLine, ParsedLyrics } from "@/lib/lrc";
import { parseLrc } from "@/lib/lrc";
import { answerLyrics, LYRICS_WINDOW_MS, lyricsToWire, noLyrics, type LyricsPort, type LyricsTarget } from "./lyrics";
import { executeRpc, type RpcPort } from "./rpc";
import { ytRef } from "./testKit";

const track = ytRef("vid1");

const parsed = (lines: LyricLine[], over: Partial<ParsedLyrics> = {}): ParsedLyrics => ({
  lines,
  synced: lines.some((l) => l.timeMs !== null),
  wordSynced: lines.some((l) => l.words),
  offsetMs: 0,
  ...over,
});

describe("lyricsToWire", () => {
  it("maps synced lines to track-time starts", () => {
    const out = lyricsToWire(
      track,
      parsed([
        { timeMs: 18_000, text: "We're no strangers to love", endMs: 22_300 },
        { timeMs: 22_300, text: "You know the rules" },
      ]),
    );
    expect(out).toEqual({
      track,
      found: true,
      synced: true,
      source: null,
      lines: [
        { startMs: 18_000, text: "We're no strangers to love", words: null },
        { startMs: 22_300, text: "You know the rules", words: null },
      ],
    });
  });

  it("carries word timing: each word ends where the next begins, the last where its line does", () => {
    const out = lyricsToWire(
      track,
      parsed([
        {
          timeMs: 1000,
          text: "hello big world",
          endMs: 3000,
          words: [
            { timeMs: 1000, text: "hello " },
            { timeMs: 1400, text: "big " },
            { timeMs: 2000, text: "world" },
          ],
        },
        { timeMs: 3000, text: "next" },
      ]),
    );
    expect(out.lines[0]!.words).toEqual([
      { startMs: 1000, endMs: 1400, text: "hello " },
      { startMs: 1400, endMs: 2000, text: "big " },
      { startMs: 2000, endMs: 3000, text: "world" },
    ]);
    expect(out.lines[1]!.words).toBeNull();
  });

  it("closes the last word of the last line 4 s after it starts, like the panel", () => {
    const out = lyricsToWire(
      track,
      parsed([{ timeMs: 5000, text: "end", words: [{ timeMs: 5000, text: "end" }] }]),
    );
    expect(out.lines[0]!.words).toEqual([{ startMs: 5000, endMs: 9000, text: "end" }]);
  });

  it("sends plain lyrics unsynced, with null starts and no words", () => {
    const out = lyricsToWire(track, parseLrc("First line\nSecond line"));
    expect(out).toMatchObject({ found: true, synced: false });
    expect(out.lines).toEqual([
      { startMs: null, text: "First line", words: null },
      { startMs: null, text: "Second line", words: null },
    ]);
  });

  it("sends a half-timed list unsynced rather than half-timed", () => {
    const out = lyricsToWire(
      track,
      parsed([
        { timeMs: 1000, text: "timed", words: [{ timeMs: 1000, text: "timed" }] },
        { timeMs: null, text: "untimed" },
      ]),
    );
    expect(out.synced).toBe(false);
    expect(out.lines.map((l) => l.startMs)).toEqual([null, null]);
    expect(out.lines.map((l) => l.words)).toEqual([null, null]);
  });

  it("applies the LRC [offset:] the panel adds to its clock, never below 0", () => {
    const out = lyricsToWire(
      track,
      parsed(
        [
          { timeMs: 100, text: "early", words: [{ timeMs: 100, text: "early" }], endMs: 1000 },
          { timeMs: 1000, text: "later" },
        ],
        { offsetMs: -300 },
      ),
    );
    expect(out.lines.map((l) => l.startMs)).toEqual([0, 700]);
    expect(out.lines[0]!.words).toEqual([{ startMs: 0, endMs: 700, text: "early" }]);
    const late = lyricsToWire(track, parsed([{ timeMs: 1000, text: "x" }], { offsetMs: 250 }));
    expect(late.lines[0]!.startMs).toBe(1250);
  });

  it("does not shift unsynced lyrics by an offset", () => {
    const out = lyricsToWire(track, parsed([{ timeMs: null, text: "x" }], { offsetMs: 500 }));
    expect(out.lines[0]!.startMs).toBeNull();
  });

  it("trims text", () => {
    const out = lyricsToWire(track, parsed([{ timeMs: 0, text: "  padded  " }]));
    expect(out.lines[0]!.text).toBe("padded");
  });

  it("is found:false for null, empty, or all-blank lyrics — still naming the track", () => {
    expect(lyricsToWire(track, null)).toEqual(noLyrics(track));
    expect(lyricsToWire(track, parsed([]))).toEqual(noLyrics(track));
    expect(lyricsToWire(track, parsed([{ timeMs: 0, text: "   " }]))).toEqual(noLyrics(track));
    expect(noLyrics(track)).toEqual({ track, found: false, synced: false, source: null, lines: [] });
  });

  it("keeps an instrumental gap's blank line once there is real text", () => {
    const out = lyricsToWire(track, parsed([{ timeMs: 0, text: "la" }, { timeMs: 5000, text: "" }]));
    expect(out.lines[1]).toEqual({ startMs: 5000, text: "", words: null });
  });
});

function port(over: Partial<LyricsPort> = {}) {
  const target: LyricsTarget = {
    track,
    key: "ytmusic:vid1",
    title: "Song vid1",
    artist: "Artist",
    durationSecs: 200,
  };
  const resolve = vi.fn(async () => parsed([{ timeMs: 0, text: "la" }]) as ParsedLyrics | null);
  return { current: () => target, resolve, ...over } as LyricsPort & { resolve: typeof resolve };
}

describe("answerLyrics", () => {
  afterEach(() => vi.useRealTimers());

  it("answers found:false with no track when nothing is playing, without looking anything up", async () => {
    const p = port({ current: () => null });
    expect(await answerLyrics(p)).toEqual(noLyrics(null));
    expect(p.resolve).not.toHaveBeenCalled();
  });

  it("answers the current track's lyrics", async () => {
    const p = port();
    const out = await answerLyrics(p);
    expect(out).toMatchObject({ track, found: true, synced: true });
    expect(p.resolve).toHaveBeenCalledOnce();
  });

  it("does not look up (or cache a miss for) a track whose title hasn't resolved yet", async () => {
    const p = port({
      current: () => ({ track, key: "k", title: null, artist: null, durationSecs: null }),
    });
    expect(await answerLyrics(p)).toEqual(noLyrics(track));
    expect(p.resolve).not.toHaveBeenCalled();
  });

  it("answers found:false after the window when the lookup is slow, and lets it finish", async () => {
    vi.useFakeTimers();
    let finish!: (v: ParsedLyrics | null) => void;
    const slow = new Promise<ParsedLyrics | null>((r) => (finish = r));
    const p = port({ resolve: vi.fn(() => slow) });
    let answer: unknown;
    void answerLyrics(p).then((a) => (answer = a));

    // The contract's budget: answer within ~10 s of a 15 s hub timeout.
    expect(LYRICS_WINDOW_MS).toBe(10_000);
    await vi.advanceTimersByTimeAsync(9_999);
    expect(answer, "still waiting just inside the window").toBeUndefined();
    await vi.advanceTimersByTimeAsync(1);
    expect(answer).toEqual(noLyrics(track));

    // The lookup was not cancelled: it completes (and fills the resolver's
    // cache) so the controller's next ask is served from there.
    finish(parsed([{ timeMs: 0, text: "found late" }]));
    await expect(slow).resolves.toBeTruthy();
  });

  it("answers as soon as the lookup does, not at the window", async () => {
    vi.useFakeTimers();
    const p = port();
    let answer: unknown;
    void answerLyrics(p).then((a) => (answer = a));
    await vi.advanceTimersByTimeAsync(0);
    expect(answer).toMatchObject({ found: true });
  });

  it("treats a failing lookup as found:false", async () => {
    const p = port({ resolve: vi.fn(async () => Promise.reject(new Error("net"))) });
    expect(await answerLyrics(p)).toEqual(noLyrics(track));
    const sync = port({
      resolve: vi.fn(() => {
        throw new Error("sync");
      }),
    });
    expect(await answerLyrics(sync)).toEqual(noLyrics(track));
  });

  it("honours a custom window", async () => {
    vi.useFakeTimers();
    const p = port({ resolve: vi.fn(() => new Promise<ParsedLyrics | null>(() => {})) });
    let answer: unknown;
    void answerLyrics(p, 50).then((a) => (answer = a));
    await vi.advanceTimersByTimeAsync(50);
    expect(answer).toEqual(noLyrics(track));
  });
});

describe("the lyrics rpc", () => {
  const base = (lyrics: LyricsPort) =>
    ({
      deviceId: () => "d",
      ytSignedIn: async () => false, // never consulted for lyrics
      search: async () => [],
      suggest: async () => [],
      exploreTracks: async () => [],
      artistPage: async () => [],
      localTracks: async () => [],
      forgetSignIn: () => {},
      lyrics,
    }) satisfies RpcPort;

  it("answers without a YouTube Music session", async () => {
    const out = await executeRpc("lyrics", {}, base(port()));
    expect(out).toMatchObject({ ok: true, result: { found: true, track } });
  });
});
