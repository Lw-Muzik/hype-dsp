import { describe, expect, it, vi } from "vitest";
import { createInboundHandler } from "./bridge";
import type { PlayerPort } from "./commands";
import type { Frame } from "./protocol";
import { QueueUidTracker } from "./queueUids";
import type { RpcPort } from "./rpc";
import { DEVICE, ytRef } from "./testKit";

function setup() {
  const sent: Frame[] = [];
  const player = {
    deviceId: () => DEVICE,
    findLocal: async () => null,
    snapshot: () => ({ playing: false, paused: false, seekable: true, queue: [], order: [], orderPos: -1 }),
    playItems: vi.fn(),
    playRadio: vi.fn(),
    startAt: vi.fn(),
    enqueue: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    jumpTo: vi.fn(),
    next: vi.fn(),
    prev: vi.fn(),
    seek: vi.fn(),
    stop: vi.fn(async () => {}),
    setVolume: vi.fn(),
    setShuffle: vi.fn(),
    setRepeat: vi.fn(),
  } satisfies PlayerPort;
  const rpc: RpcPort = {
    deviceId: () => DEVICE,
    ytSignedIn: async () => false,
    search: async () => [],
    suggest: async () => [],
    exploreTracks: async () => [],
    artistPage: async () => [],
    localTracks: async () => [],
    forgetSignIn: () => {},
  };
  const onDevices = vi.fn();
  const handle = createInboundHandler({
    player,
    rpc,
    uids: new QueueUidTracker(),
    send: async (f) => void sent.push(f),
    onDevices,
  });
  const settle = () => new Promise((r) => setTimeout(r, 0));
  return { handle, sent, player, onDevices, settle };
}

describe("createInboundHandler", () => {
  it("executes a cmd and acks it ok with the same id", async () => {
    const { handle, sent, player, settle } = setup();
    handle({ t: "cmd", id: "c-1", cmd: { type: "play", index: 0, items: [ytRef("a")] } });
    await settle();
    expect(player.playItems).toHaveBeenCalledOnce();
    expect(sent).toEqual([{ t: "ack", id: "c-1", ok: true }]);
  });

  it("acks a refused cmd with its error", async () => {
    const { handle, sent, settle } = setup();
    handle({ t: "cmd", id: "c-2", cmd: { type: "play", index: 0, items: [] } });
    await settle();
    expect(sent).toEqual([{ t: "ack", id: "c-2", ok: false, error: "nothing_playable" }]);
  });

  it("answers an undecodable cmd with `unsupported` instead of leaving it to time out", async () => {
    const { handle, sent, player, settle } = setup();
    handle({ t: "cmd", id: "c-3", cmd: { type: "teleport" } });
    await settle();
    expect(player.playItems).not.toHaveBeenCalled();
    expect(sent).toEqual([{ t: "ack", id: "c-3", ok: false, error: "unsupported" }]);
  });

  it("answers an undecodable rpc with `unsupported`", async () => {
    const { handle, sent, settle } = setup();
    handle({ t: "rpc", id: "r-1", method: "levitate", params: {} });
    await settle();
    expect(sent).toEqual([{ t: "rpc_result", id: "r-1", ok: false, error: "unsupported" }]);
  });

  it("ignores garbage with no id", async () => {
    const { handle, sent, settle } = setup();
    handle(null);
    handle({ t: "cmd" });
    await settle();
    expect(sent).toEqual([]);
  });

  it("answers an rpc with an rpc_result carrying its id", async () => {
    const { handle, sent, settle } = setup();
    handle({ t: "rpc", id: "r-2", method: "search", params: { query: "x", filter: "all" } });
    await settle();
    expect(sent).toEqual([{ t: "rpc_result", id: "r-2", ok: false, error: "not_signed_in" }]);
  });

  it("feeds welcome and devices frames to the device list", () => {
    const { handle, onDevices } = setup();
    handle({ t: "devices", devices: [], activeDeviceId: "d-1" });
    expect(onDevices).toHaveBeenCalledWith([], "d-1");
  });

  it("acks commands in arrival order", async () => {
    const { handle, sent, settle } = setup();
    handle({ t: "cmd", id: "1", cmd: { type: "pause" } });
    handle({ t: "cmd", id: "2", cmd: { type: "next" } });
    handle({ t: "cmd", id: "3", cmd: { type: "prev" } });
    await settle();
    expect(sent.map((f) => (f as { id: string }).id)).toEqual(["1", "2", "3"]);
  });
});
