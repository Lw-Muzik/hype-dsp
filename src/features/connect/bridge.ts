/**
 * Inbound frames → the player. `cmd`s run one at a time and are acked when
 * accepted; `rpc`s answer as soon as their lookup finishes; `welcome` and
 * `devices` feed the device list.
 */
import { CommandQueue, executeCommand, type PlayerPort } from "./commands";
import { decodeFrame, type DeviceInfo, type Frame } from "./protocol";
import type { QueueUidTracker } from "./queueUids";
import { executeRpc, type RpcPort } from "./rpc";

export interface BridgeDeps {
  player: PlayerPort;
  rpc: RpcPort;
  uids: QueueUidTracker;
  send(frame: Frame): Promise<void>;
  onDevices?(devices: DeviceInfo[], activeDeviceId: string | null): void;
}

/** `{t, id}` of a frame that didn't decode, so it can still be answered. */
function idOf(raw: unknown): { t: unknown; id: string } | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { t, id } = raw as { t?: unknown; id?: unknown };
  return typeof id === "string" ? { t, id } : null;
}

export function createInboundHandler(deps: BridgeDeps): (raw: unknown) => void {
  const commands = new CommandQueue();
  const send = (frame: Frame) => deps.send(frame).catch(() => {});

  return (raw) => {
    const frame = decodeFrame(raw);
    if (!frame) {
      // Rust already validated it, so this is a version skew between the two
      // halves; answer rather than leave the controller waiting for a timeout.
      const head = idOf(raw);
      if (head?.t === "cmd") void send({ t: "ack", id: head.id, ok: false, error: "unsupported" });
      if (head?.t === "rpc") {
        void send({ t: "rpc_result", id: head.id, ok: false, error: "unsupported" });
      }
      return;
    }
    switch (frame.t) {
      case "cmd": {
        const { id, cmd } = frame;
        void commands.run(async () => {
          const outcome = await executeCommand(cmd, deps.player, deps.uids);
          await send(
            outcome.ok ? { t: "ack", id, ok: true } : { t: "ack", id, ok: false, error: outcome.error },
          );
        });
        return;
      }
      case "rpc": {
        const { id, method, params } = frame;
        void executeRpc(method, params, deps.rpc).then((outcome) =>
          send(
            outcome.ok
              ? { t: "rpc_result", id, ok: true, result: outcome.result }
              : { t: "rpc_result", id, ok: false, error: outcome.error },
          ),
        );
        return;
      }
      case "welcome":
      case "devices":
        deps.onDevices?.(frame.devices, frame.activeDeviceId);
        return;
      default:
        return;
    }
  };
}
