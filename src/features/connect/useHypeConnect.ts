import { useEffect } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  connectSend,
  connectStatus,
  onConnectInbound,
  onConnectStatus,
  onEngineFrame,
} from "@/lib/ipc";
import type { ConnectStatus } from "@/lib/ipc";
import { useAccountStore } from "@/stores/account";
import { confirmAsk } from "@/stores/confirm";
import { useEngineStore } from "@/stores/engine";
import { toast } from "@/stores/toast";
import { createInboundHandler } from "./bridge";
import { executeCommand } from "./commands";
import { ClockSync, HubClock } from "./hubClock";
import { appRpcPort, enginePlayerPort } from "./ports";
import { QueueUidTracker } from "./queueUids";
import { queueItemToRef } from "./refs";
import { configureRemote } from "./remote/control";
import { configureRouter, Router } from "./remote/router";
import { RemoteSession } from "./remote/session";
import { useRemoteStore } from "./remote/store";
import { Takeover } from "./remote/takeover";
import { moveTo } from "./remote/transfer";
import {
  buildPlayerState,
  createStatePublisher,
  localStorageSeqStore,
  type EngineView,
} from "./statePublisher";
import { useConnectStore } from "./store";
import { PositionExtrapolator } from "./viz/position";
import { VizStreamer } from "./viz/streamer";

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

/** Playing and not paused: what the visualiser stream and its position need. */
const isAudible = (s: { playing: boolean; paused: boolean }): boolean => s.playing && !s.paused;

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

    // Hub time (for a viz frame's `heardAt`): ping/pong on every welcome and
    // every ~10 s, the fastest of the last five round trips wins.
    const hubClock = new HubClock();
    const clockSync = new ClockSync({
      clock: hubClock,
      sendPing: (id) => connectSend({ t: "ping", id }),
      now: () => Date.now(),
      setInterval: (fn, ms) => window.setInterval(fn, ms),
      clearInterval: (handle) => window.clearInterval(handle as number),
    });

    // v1.2 — this app as a remote, and the one-player-at-a-time rules.
    const playerPort = enginePlayerPort(getDeviceId);
    const pauseLocal = () => useEngineStore.getState().handleMediaCommand("pause", null);
    const takeover = new Takeover({
      now: () => Date.now(),
      setTimeout: (fn, ms) => window.setTimeout(fn, ms),
      clearTimeout: (h) => window.clearTimeout(h as number),
      sendSelect: (id) => void connectSend({ t: "select", deviceId: id }).catch(() => {}),
      pauseLocal,
    });
    const session = new RemoteSession({
      hubNow: () => hubClock.nowMs(),
      send: connectSend,
      setTimeout: (fn, ms) => window.setTimeout(fn, ms),
      clearTimeout: (h) => window.clearTimeout(h as number),
      onChange: (view) => useRemoteStore.getState().setView(view),
      onTargetLost: (d) => toast.info(`${d.name} went offline`),
    });
    configureRouter(
      new Router({
        target: () => session.target,
        toRef: (item) => queueItemToRef(item, deviceId),
        cmd: (to, command) => session.cmd(to, command),
        confirm: confirmAsk,
        info: (m) => toast.info(m),
        error: (m) => toast.error(m),
      }),
    );
    configureRemote({
      view: () => session.view(),
      cmd: (to, command) => session.cmd(to, command),
      hubNow: () => hubClock.nowMs(),
      moveTo: (target) =>
        moveTo(target, {
          selfId: getDeviceId,
          sourceId: () => session.target?.id ?? deviceId,
          stateOf: (id) => session.stateOf(id),
          localState: () => buildPlayerState(useEngineStore.getState(), deviceId, uids),
          hubNow: () => hubClock.nowMs(),
          deviceName: (id) =>
            id === deviceId ? "this computer" : (session.players.find((p) => p.id === id)?.name ?? "that device"),
          cmd: (to, command) => session.cmd(to, command),
          select: (id) => session.select(id),
          playHere: async (cmd) => {
            const outcome = await executeCommand(cmd, playerPort, uids);
            return outcome.ok ? { ok: true } : { ok: false, error: outcome.error };
          },
          pauseLocal,
          info: (m) => toast.info(m),
          error: (m) => toast.error(m),
        }),
    });
    takeover.setPlaying(isAudible(useEngineStore.getState()));
    const syncPlayers = (active: string | null) =>
      useRemoteStore.getState().setPlayers(session.players, active, deviceId);

    // Position between the engine's ~10/s progress events.
    const position = new PositionExtrapolator();
    const syncPosition = (s: ReturnType<typeof useEngineStore.getState>) =>
      position.sync(s.positionSecs * 1000, performance.now(), isAudible(s) && !s.buffering);
    syncPosition(useEngineStore.getState());

    // Engine spectra are listened to only while a controller watches AND audio
    // plays. Listening here (not through the store) also keeps frames flowing
    // while the window is hidden: the store drops them then.
    const viz = new VizStreamer({
      listen: (onSpectrum) => {
        const attached = onEngineFrame((frame) => {
          if (frame.spectrum) onSpectrum(frame.spectrum);
        });
        return () => {
          void attached.then((unlisten) => unlisten()).catch(() => {});
        };
      },
      send: (frame) => connectSend({ t: "viz", frame }),
      isPlaying: () => isAudible(useEngineStore.getState()),
      positionMs: () => position.positionAt(performance.now()),
      hubNowMs: () => hubClock.nowMs(),
      nowMs: () => performance.now(),
    });
    const unsubscribeEngine = useEngineStore.subscribe((s, prev) => {
      if (
        s.positionSecs !== prev.positionSecs ||
        s.playing !== prev.playing ||
        s.paused !== prev.paused ||
        s.buffering !== prev.buffering
      ) {
        syncPosition(s);
      }
      if (isAudible(s) !== isAudible(prev)) {
        viz.refresh();
        takeover.setPlaying(isAudible(s));
      }
      // A new track: the beat detector forgets the last song's loudness.
      if (s.queueIndex !== prev.queueIndex || s.nowPlaying !== prev.nowPlaying) viz.resetAnalysis();
    });

    const handleInbound = createInboundHandler({
      player: playerPort,
      rpc: appRpcPort(getDeviceId),
      uids,
      send: connectSend,
      onDevices: (devices, active) => {
        useConnectStore.getState().setDevices(devices, active);
        takeover.onDevices(devices, active);
        session.onDevices(devices, active);
        syncPlayers(active);
      },
      onWelcomeStates: (devices, active, states) => {
        // A welcome is proof of a live socket, whichever of it and the status
        // event reaches the webview first.
        takeover.setConnected(true);
        session.setConnected(true);
        session.onWelcome(devices, active, states);
      },
      onState: (id, state) => session.onState(id, state),
      onAck: (id, ok, error) => session.onAck(id, ok, error),
      onError: (code) => takeover.onError(code),
      onWelcome: (serverTime) => hubClock.seedFromWelcome(serverTime, Date.now()),
      onPong: (id, serverTime) => clockSync.onPong(id, serverTime),
      onVizSub: (on) => viz.setSubscribed(on),
    });

    const applyStatus = (status: ConnectStatus) => {
      deviceId = status.deviceId;
      takeover.setSelfId(deviceId);
      session.setSelfId(deviceId);
      useConnectStore.getState().setStatus(status);
      const c = status.connection;
      const online = c?.state === "connected";
      takeover.setConnected(online);
      session.setConnected(online);
      if (!online) syncPlayers(null);
      // A fresh socket gets the current state at once; between sockets there
      // is nobody to tell.
      if (c?.state === "connected") {
        publisher.start();
        clockSync.start();
        viz.setOnline(true);
        // The hub's `viz_sub` can reach Rust before this listener exists (a
        // controller already watching when the app launches); Rust remembers.
        if (status.vizSubscribed) viz.setSubscribed(true);
      } else {
        publisher.stop();
        clockSync.stop();
        viz.setOnline(false);
      }
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
      clockSync.stop();
      viz.dispose();
      unsubscribeEngine();
      takeover.dispose();
      session.dispose();
      configureRouter(null);
      configureRemote(null);
      for (const un of unlisteners) un();
    };
  }, []);
}
