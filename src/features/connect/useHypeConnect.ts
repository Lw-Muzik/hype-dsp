import { useEffect } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { connectSend, connectStatus, onConnectInbound, onConnectStatus } from "@/lib/ipc";
import type { ConnectStatus } from "@/lib/ipc";
import { useAccountStore } from "@/stores/account";
import { useEngineStore } from "@/stores/engine";
import { createInboundHandler } from "./bridge";
import { appRpcPort, enginePlayerPort } from "./ports";
import { QueueUidTracker } from "./queueUids";
import {
  buildPlayerState,
  createStatePublisher,
  localStorageSeqStore,
  type EngineView,
} from "./statePublisher";
import { useConnectStore } from "./store";

/** Store changes a controller could see. Meters and spectrum (30 fps) and
 *  every DSP setting other than volume are not among them. */
function playerChanged(s: EngineView, prev: EngineView): boolean {
  return (
    s.playing !== prev.playing ||
    s.paused !== prev.paused ||
    s.nowPlaying !== prev.nowPlaying ||
    s.positionSecs !== prev.positionSecs ||
    s.durationSecs !== prev.durationSecs ||
    s.queue !== prev.queue ||
    s.queueIndex !== prev.queueIndex ||
    s.order !== prev.order ||
    s.orderPos !== prev.orderPos ||
    s.shuffle !== prev.shuffle ||
    s.repeat !== prev.repeat ||
    s.state.masterVolume !== prev.state.masterVolume
  );
}

/**
 * Hype Connect's player half, mounted once for the app's lifetime: executes
 * the hub's commands and rpcs against the app's own player and catalog, and
 * publishes this desktop's state while connected. It never navigates — a
 * remote `play` changes what plays, not what's on screen.
 */
export function useHypeConnect(): void {
  useEffect(() => {
    let cancelled = false;
    let heardEvent = false;
    let deviceId = "";
    const unlisteners: UnlistenFn[] = [];
    const uids = new QueueUidTracker();
    const getDeviceId = () => deviceId;

    const publisher = createStatePublisher({
      build: () => buildPlayerState(useEngineStore.getState(), deviceId, uids),
      subscribe: (onChange) =>
        useEngineStore.subscribe((s, prev) => {
          if (playerChanged(s, prev)) onChange();
        }),
      send: (state) => connectSend({ t: "state", state }),
      now: () => Date.now(),
      seqStore: localStorageSeqStore(),
    });

    const handleInbound = createInboundHandler({
      player: enginePlayerPort(getDeviceId),
      rpc: appRpcPort(getDeviceId),
      uids,
      send: connectSend,
      onDevices: (devices, active) => useConnectStore.getState().setDevices(devices, active),
    });

    const applyStatus = (status: ConnectStatus) => {
      deviceId = status.deviceId;
      useConnectStore.getState().setStatus(status);
      const c = status.connection;
      // A fresh socket gets the current state at once; between sockets there
      // is nobody to tell.
      if (c?.state === "connected") publisher.start();
      else publisher.stop();
      if (c?.state === "stopped" && c.reason === "unlinked") {
        // Rust already dropped the revoked session; seeing it gone, SessionGate
        // bootstraps a fresh anonymous one.
        void useAccountStore.getState().refresh();
      }
    };

    const keep = (p: Promise<UnlistenFn>) =>
      p.then((un) => (cancelled ? un() : unlisteners.push(un))).catch(() => {});

    keep(
      onConnectStatus((s) => {
        heardEvent = true;
        applyStatus(s);
      }),
    );
    keep(onConnectInbound(handleInbound));
    connectStatus()
      .then((s) => {
        if (!cancelled && !heardEvent) applyStatus(s);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      publisher.stop();
      for (const un of unlisteners) un();
    };
  }, []);
}
