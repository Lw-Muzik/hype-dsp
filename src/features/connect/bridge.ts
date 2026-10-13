/**
 * Inbound frames → the player. `cmd`s run one at a time and are acked when
 * accepted; `rpc`s answer as soon as their lookup finishes; `welcome` and
 * `devices` feed the device list (and, v1.2, the remote session and the
 * claim/yield rules, with `state`, `ack` and `error`); `pong` feeds the hub clock and the hub's
 * `viz_sub` starts or stops the visualiser stream (v1.1).
 */
import { CommandQueue, executeCommand, type PlayerPort } from "./commands";
import { decodeFrame, type AckError, type DeviceInfo, type Frame, type PlayerState } from "./protocol";
import type { QueueUidTracker } from "./queueUids";
import { executeRpc, type RpcPort } from "./rpc";

export interface BridgeDeps {
  player: PlayerPort;
  rpc: RpcPort;
  uids: QueueUidTracker;
  send(frame: Frame): Promise<void>;
  onDevices?(devices: DeviceInfo[], activeDeviceId: string | null): void;
  /** The hub's clock at `welcome` (a first guess before any `pong`). */
  onWelcome?(serverTime: number): void;
  /** `welcome` in full: the devices, who is active and every online player's
   *  latest state (v1.2: this app can be a remote). Runs before `onDevices`. */
  onWelcomeStates?(
    devices: DeviceInfo[],
    activeDeviceId: string | null,
    states: Record<string, PlayerState>,
  ): void;
  /** Another player's state (v1.2). */
  onState?(deviceId: string | undefined, state: PlayerState): void;
  /** The answer to a `cmd` this app sent as a remote (v1.2). */
  onAck?(id: string, ok: boolean, error?: AckError): void;
  /** `error` from the hub (v1.2: `not_a_player` ends a claim). */
  onError?(code: string, message: string): void;
  onPong?(id: string, serverTime: number): void;
  /** The hub's subscriber count went 0 → 1 (`true`) or 1 → 0 (`false`). */
  onVizSub?(on: boolean): void;
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
        deps.onWelcome?.(frame.serverTime);
        deps.onWelcomeStates?.(frame.devices, frame.activeDeviceId, frame.states);
        deps.onDevices?.(frame.devices, frame.activeDeviceId);
        return;
      case "devices":
        deps.onDevices?.(frame.devices, frame.activeDeviceId);
        return;
      case "state":
        deps.onState?.(frame.deviceId, frame.state);
        return;
      case "ack":
        deps.onAck?.(frame.id, frame.ok, frame.error);
        return;
      case "error":
        deps.onError?.(frame.code, frame.message);
        return;
      case "pong":
        deps.onPong?.(frame.id, frame.serverTime);
        return;
      case "viz_sub":
        deps.onVizSub?.(frame.on);
        return;
      default:
        return;
    }
  };
}
