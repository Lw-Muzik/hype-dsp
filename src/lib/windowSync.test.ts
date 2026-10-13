import { afterEach, describe, expect, it, vi } from "vitest";
import { createStore, type StoreApi } from "zustand/vanilla";
import { applySynced, diffSynced, snapshotSynced, type SyncSpec } from "./windowSync";

/**
 * The Settings window and the main window are separate JS contexts. These
 * tests load one copy of `windowSync` per simulated window (each with its own
 * label) on an in-memory event bus that delivers asynchronously, like Tauri.
 */

type Listener = { label: string; event: string; cb: (e: { payload: unknown }) => void };

class Bus {
  listeners: Listener[] = [];
  sent: { from: string; event: string; to?: string; payload: unknown }[] = [];

  api(label: string) {
    const deliver = (event: string, payload: unknown, to?: string) => {
      this.sent.push({ from: label, event, to, payload });
      const targets = this.listeners.filter(
        (l) => l.event === event && (to === undefined || l.label === to),
      );
      queueMicrotask(() => targets.forEach((l) => l.cb({ payload })));
    };
    return {
      emit: async (event: string, payload: unknown) => deliver(event, payload),
      emitTo: async (to: string, event: string, payload: unknown) => deliver(event, payload, to),
      listen: async (event: string, cb: Listener["cb"]) => {
        const l = { label, event, cb };
        this.listeners.push(l);
        return () => {
          this.listeners = this.listeners.filter((x) => x !== l);
        };
      },
    };
  }
}

const flush = () => new Promise((r) => setTimeout(r, 0));

async function loadWindow(bus: Bus, label: string) {
  vi.resetModules();
  vi.doMock("@/lib/windowLabel", () => ({
    WINDOW_LABEL: label,
    MAIN_WINDOW: "main",
    SETTINGS_WINDOW: "settings",
    isMainWindow: label === "main",
    isSettingsWindow: label === "settings",
  }));
  vi.doMock("@tauri-apps/api/event", () => bus.api(label));
  return import("./windowSync");
}

interface Engine {
  state: { playback: { gapless: boolean; crossfadeSecs: number }; scope: string; eq: number[] };
  nowPlaying: string | null;
  meters: number;
}

const engineSpec: SyncSpec<Engine> = {
  keys: ["nowPlaying"],
  merge: { state: ["playback", "scope"] },
};

const engine = (over: Partial<Engine["state"]> = {}): StoreApi<Engine> =>
  createStore<Engine>(() => ({
    state: { playback: { gapless: true, crossfadeSecs: 0 }, scope: "all", eq: [0, 0], ...over },
    nowPlaying: null,
    meters: 0,
  }));

afterEach(() => {
  vi.doUnmock("@/lib/windowLabel");
  vi.doUnmock("@tauri-apps/api/event");
});

describe("diff / snapshot / apply", () => {
  it("diffs only synced keys and only the listed fields of merge keys", () => {
    const prev = engine().getState();
    const next: Engine = {
      ...prev,
      meters: 9,
      state: { ...prev.state, eq: [3, 3], playback: { gapless: false, crossfadeSecs: 0 } },
    };
    expect(diffSynced(prev, next, engineSpec)).toEqual({
      state: { playback: { gapless: false, crossfadeSecs: 0 } },
    });
    expect(diffSynced(prev, { ...prev, meters: 1 }, engineSpec)).toBeNull();
  });

  it("snapshots every synced key and nothing else", () => {
    const s = engine({ scope: "only" }).getState();
    expect(snapshotSynced(s, engineSpec)).toEqual({
      nowPlaying: null,
      state: { playback: { gapless: true, crossfadeSecs: 0 }, scope: "only" },
    });
  });

  it("merges merge keys into the receiver's object", () => {
    const s = engine({ eq: [5, 5] }).getState();
    const out = applySynced(s, { state: { scope: "except" } }, engineSpec);
    expect(out.state).toEqual({ ...s.state, scope: "except" });
  });
});

describe("two windows", () => {
  it("mirrors a change and never echoes it back", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const mainStore = engine();
    await main.syncStore(mainStore, "engine", engineSpec);
    const settings = await loadWindow(bus, "settings");
    const settingsStore = engine();
    await settings.syncStore(settingsStore, "engine", engineSpec);

    settingsStore.setState((s) => ({
      state: { ...s.state, playback: { gapless: true, crossfadeSecs: 6 } },
    }));
    await flush();

    expect(mainStore.getState().state.playback.crossfadeSecs).toBe(6);
    const syncs = bus.sent.filter((m) => m.event === "hm://sync");
    expect(syncs).toHaveLength(1); // main applied it without re-emitting
    expect(syncs[0]?.from).toBe("settings");
  });

  it("does not overwrite fields the receiver changed meanwhile, or unsynced ones", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const mainStore = engine();
    await main.syncStore(mainStore, "engine", engineSpec);
    const settings = await loadWindow(bus, "settings");
    const settingsStore = engine();
    await settings.syncStore(settingsStore, "engine", engineSpec);

    // Main drags the EQ (unsynced) and changes scope; Settings changes playback.
    mainStore.setState((s) => ({ state: { ...s.state, eq: [4, 4], scope: "only" } }));
    settingsStore.setState((s) => ({
      state: { ...s.state, playback: { gapless: false, crossfadeSecs: 0 } },
    }));
    await flush();

    expect(mainStore.getState().state).toEqual({
      eq: [4, 4],
      scope: "only",
      playback: { gapless: false, crossfadeSecs: 0 },
    });
    expect(settingsStore.getState().state.scope).toBe("only");
    expect(settingsStore.getState().state.eq).toEqual([0, 0]);
  });

  it("starts a new window from the other window's state, not its defaults", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const mainStore = engine({ scope: "except" });
    mainStore.setState((s) => ({
      nowPlaying: "Song",
      state: { ...s.state, playback: { gapless: false, crossfadeSecs: 8 } },
    }));
    await main.syncStore(mainStore, "engine", engineSpec);

    const settings = await loadWindow(bus, "settings");
    const settingsStore = engine();
    await settings.syncStore(settingsStore, "engine", engineSpec);
    await settings.requestSnapshot();
    await flush();

    expect(settingsStore.getState().nowPlaying).toBe("Song");
    expect(settingsStore.getState().state.playback).toEqual({ gapless: false, crossfadeSecs: 8 });
    expect(settingsStore.getState().state.scope).toBe("except");
    // The snapshot went only to the window that asked.
    const snap = bus.sent.find((m) => (m.payload as { snapshot?: boolean })?.snapshot);
    expect(snap?.to).toBe("settings");
  });

  it("a late snapshot does not roll back what the new window already changed", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const mainStore = engine({ scope: "only" });
    await main.syncStore(mainStore, "engine", engineSpec);

    const settings = await loadWindow(bus, "settings");
    const settingsStore = engine();
    await settings.syncStore(settingsStore, "engine", engineSpec);
    // Not awaited: the user flips gapless in Settings before the snapshot lands.
    void settings.requestSnapshot();
    settingsStore.setState((s) => ({
      state: { ...s.state, playback: { gapless: false, crossfadeSecs: 0 } },
    }));
    await flush();

    expect(settingsStore.getState().state.playback.gapless).toBe(false);
    expect(settingsStore.getState().state.scope).toBe("only");
    expect(mainStore.getState().state.playback.gapless).toBe(false);
  });

  it("keeps stores with different names apart", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const a = createStore(() => ({ v: 1 }));
    await main.syncStore(a, "a", { keys: ["v"] });
    const settings = await loadWindow(bus, "settings");
    const b = createStore(() => ({ v: 1 }));
    await settings.syncStore(b, "b", { keys: ["v"] });

    b.setState({ v: 2 });
    await flush();
    expect(a.getState().v).toBe(1);
  });

  it("stops syncing after the returned function runs", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const mainStore = createStore(() => ({ v: 1 }));
    const stop = await main.syncStore(mainStore, "s", { keys: ["v"] });
    const settings = await loadWindow(bus, "settings");
    const settingsStore = createStore(() => ({ v: 1 }));
    await settings.syncStore(settingsStore, "s", { keys: ["v"] });

    stop();
    settingsStore.setState({ v: 3 });
    await flush();
    expect(mainStore.getState().v).toBe(1);
  });
});

describe("forwarding to the main window", () => {
  it("runs a forwarded action in main with its arguments", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const play = vi.fn();
    await main.serveMainInvocations({ "engine.playCloudList": play });
    const settings = await loadWindow(bus, "settings");

    settings.forwardToMain("engine.playCloudList", [[{ id: "x" }], 0]);
    await flush();
    expect(play).toHaveBeenCalledWith([{ id: "x" }], 0);
  });

  it("ignores unknown actions and survives a throwing or rejecting handler", async () => {
    const bus = new Bus();
    const main = await loadWindow(bus, "main");
    const ok = vi.fn();
    await main.serveMainInvocations({
      boom: () => {
        throw new Error("x");
      },
      reject: () => Promise.reject(new Error("y")),
      ok,
    });
    const settings = await loadWindow(bus, "settings");

    settings.forwardToMain("nope", []);
    settings.forwardToMain("boom", []);
    settings.forwardToMain("reject", []);
    settings.forwardToMain("ok", [1]);
    await flush();
    expect(ok).toHaveBeenCalledWith(1);
  });
});
