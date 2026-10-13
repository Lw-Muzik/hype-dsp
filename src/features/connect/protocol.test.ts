import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { decodeFrame } from "./protocol";

const DIR = join(process.cwd(), "connect-protocol", "fixtures");
const files = readdirSync(DIR).filter((f) => f.endsWith(".json"));
const read = (f: string): unknown => JSON.parse(readFileSync(join(DIR, f), "utf8"));
/** REST bodies aren't WebSocket frames. */
const isFrameFixture = (f: string) => !f.startsWith("rest.");

describe("fixtures", () => {
  it("finds the shared fixture set", () => {
    // 56 fixtures in the v1.1 contract, 6 of them REST bodies.
    expect(files.length).toBe(56);
    expect(files.filter(isFrameFixture).length).toBe(50);
  });

  describe.each(files.filter(isFrameFixture))("%s", (file) => {
    it("decodes and round-trips to the same JSON", () => {
      const raw = read(file);
      const frame = decodeFrame(raw);
      expect(frame).not.toBeNull();
      expect(JSON.parse(JSON.stringify(frame))).toEqual(raw);
    });
  });
});

describe("decodeFrame rejects what the contract forbids", () => {
  const cmd = () => structuredClone(read("cmd.play.to.json")) as Record<string, any>;

  it("non-objects and unknown frame types", () => {
    expect(decodeFrame(null)).toBeNull();
    expect(decodeFrame("x")).toBeNull();
    expect(decodeFrame([])).toBeNull();
    expect(decodeFrame({ t: "nope" })).toBeNull();
  });

  it("a cmd with an unknown command type", () => {
    const f = cmd();
    f.cmd.type = "explode";
    expect(decodeFrame(f)).toBeNull();
  });

  it("a track with an unknown src", () => {
    const f = cmd();
    f.cmd.items[0].src = "spotify";
    expect(decodeFrame(f)).toBeNull();
  });

  it("a cmd without an id", () => {
    const f = cmd();
    delete f.id;
    expect(decodeFrame(f)).toBeNull();
  });

  it("a non-finite number where a number is required", () => {
    const f = cmd();
    f.cmd.index = "0";
    expect(decodeFrame(f)).toBeNull();
  });

  it("a repeat mode outside off/all/one", () => {
    const f = structuredClone(read("cmd.repeat.json")) as Record<string, any>;
    f.cmd.mode = "twice";
    expect(decodeFrame(f)).toBeNull();
  });

  it("an ack with an unknown error code", () => {
    const f = structuredClone(read("ack.error.json")) as Record<string, any>;
    f.error = "kaboom";
    expect(decodeFrame(f)).toBeNull();
  });
});

describe("v1.1", () => {
  const clone = (f: string) => structuredClone(read(f)) as Record<string, any>;

  it("keeps the v1.1 caps and drops caps it doesn't know", () => {
    const hello = decodeFrame(read("hello.mobile.v11.json"));
    expect(hello).toMatchObject({
      t: "hello",
      device: { caps: ["player", "yt", "local", "lyrics", "viz"] },
    });

    const welcome = clone("welcome.json");
    welcome.devices[0].caps = ["player", "hologram", "viz", 7, null];
    const w = decodeFrame(welcome);
    expect(w?.t === "welcome" && w.devices[0]?.caps).toEqual(["player", "viz"]);
    const devices = clone("devices.json");
    devices.devices[0].caps = ["hologram"];
    const decoded = decodeFrame(devices);
    expect(decoded?.t === "devices" && decoded.devices[0]?.caps).toEqual([]);
  });

  it("still rejects a caps field that isn't a list", () => {
    const devices = clone("devices.json");
    devices.devices[0].caps = "player";
    expect(decodeFrame(devices)).toBeNull();
  });

  it("decodes the lyrics rpc", () => {
    expect(decodeFrame(read("rpc.lyrics.from.json"))).toMatchObject({
      t: "rpc",
      method: "lyrics",
      params: {},
    });
  });

  it("keeps a viz_sub's direction", () => {
    expect(decodeFrame(read("viz_sub.player.on.json"))).toEqual({ t: "viz_sub", on: true });
    expect(decodeFrame(read("viz_sub.to.json"))).toMatchObject({ t: "viz_sub", on: true, to: expect.any(String) });
    const bad = clone("viz_sub.player.on.json");
    bad.on = "yes";
    expect(decodeFrame(bad)).toBeNull();
  });

  it("rejects a viz frame that isn't exactly 32 numeric bands", () => {
    const short = clone("viz.player.json");
    short.frame.bands.pop();
    expect(decodeFrame(short)).toBeNull();
    const long = clone("viz.player.json");
    long.frame.bands.push(0.5);
    expect(decodeFrame(long)).toBeNull();
    const text = clone("viz.player.json");
    text.frame.bands[4] = "0.5";
    expect(decodeFrame(text)).toBeNull();
    const nolevel = clone("viz.player.json");
    delete nolevel.frame.level;
    expect(decodeFrame(nolevel)).toBeNull();
  });
});
