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
    expect(files.filter(isFrameFixture).length).toBeGreaterThanOrEqual(40);
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
