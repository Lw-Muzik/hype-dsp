import { beforeEach, describe, expect, it, vi } from "vitest";

const events = vi.hoisted(() => ({
  emit: vi.fn(async (..._a: unknown[]) => {}),
  emitTo: vi.fn(async (..._a: unknown[]) => {}),
  handlers: new Map<string, (e: { payload: unknown }) => void>(),
  order: [] as string[],
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: events.emit,
  emitTo: events.emitTo,
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    events.handlers.set(event, cb);
    return () => events.handlers.delete(event);
  }),
}));

vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    appInfo: vi.fn(async () => {
      events.order.push("appInfo");
      return { name: "HypeMuzik", version: "9.9.9", engineSchema: 1 };
    }),
    engineGetState: vi.fn(async () => {
      events.order.push("engineGetState");
      throw new Error("no engine in tests");
    }),
  };
});

import { MAIN_INVOKE_EVENT, SYNC_EVENT, SYNC_REQUEST_EVENT } from "@/lib/windowSync";

/** A fresh copy of the stores and setup per test, so subscriptions don't leak. */
async function load() {
  vi.resetModules();
  const [setup, engine, library, music, ui] = await Promise.all([
    import("@/app/windowSyncSetup"),
    import("@/stores/engine"),
    import("@/stores/library"),
    import("@/stores/musicLibrary"),
    import("@/stores/ui"),
  ]);
  return {
    ...setup,
    useEngineStore: engine.useEngineStore,
    useLibraryStore: library.useLibraryStore,
    useMusicLibraryStore: music.useMusicLibraryStore,
    useUiStore: ui.useUiStore,
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  events.emit.mockClear();
  events.emitTo.mockClear();
  events.handlers.clear();
  events.order.length = 0;
});

describe("startSettingsWindowSync", () => {
  it("replaces the main-owned actions with forwarders that emit to main", async () => {
    const { startSettingsWindowSync, useEngineStore, useLibraryStore, useMusicLibraryStore } =
      await load();
    const real = {
      playPhoneList: useEngineStore.getState().playPhoneList,
      playCloudList: useEngineStore.getState().playCloudList,
      refresh: useLibraryStore.getState().refresh,
      syncPhone: useMusicLibraryStore.getState().syncPhone,
      invalidateCloud: useMusicLibraryStore.getState().invalidateCloud,
      invalidateYtMusic: useMusicLibraryStore.getState().invalidateYtMusic,
    };
    const versionBefore = useLibraryStore.getState().version;

    await startSettingsWindowSync();

    const engine = useEngineStore.getState();
    const music = useMusicLibraryStore.getState();
    expect(engine.playPhoneList).not.toBe(real.playPhoneList);
    expect(engine.playCloudList).not.toBe(real.playCloudList);
    expect(useLibraryStore.getState().refresh).not.toBe(real.refresh);
    expect(music.syncPhone).not.toBe(real.syncPhone);
    expect(music.invalidateCloud).not.toBe(real.invalidateCloud);
    expect(music.invalidateYtMusic).not.toBe(real.invalidateYtMusic);

    const device = { id: "d" } as never;
    const tracks = [{ id: "t" }] as never;
    engine.playPhoneList(device, tracks, 2);
    engine.playCloudList([] as never, 1);
    useLibraryStore.getState().refresh();
    music.syncPhone();
    music.invalidateCloud();
    music.invalidateYtMusic();

    const calls = events.emitTo.mock.calls.filter((c) => c[1] === MAIN_INVOKE_EVENT);
    expect(calls.map((c) => [c[0], (c[2] as { action: string }).action])).toEqual([
      ["main", "engine.playPhoneList"],
      ["main", "engine.playCloudList"],
      ["main", "library.refresh"],
      ["main", "musicLibrary.syncPhone"],
      ["main", "musicLibrary.invalidateCloud"],
      ["main", "musicLibrary.invalidateYtMusic"],
    ]);
    expect((calls[0]?.[2] as { args: unknown[] }).args).toEqual([device, tracks, 2]);
    // The real refresh did not run here.
    expect(useLibraryStore.getState().version).toBe(versionBefore);
  });

  it("hydrates before syncing and broadcasts nothing while starting up", async () => {
    const { startSettingsWindowSync, useUiStore } = await load();
    await startSettingsWindowSync();
    expect(events.order).toEqual(["appInfo", "engineGetState"]);
    expect(useUiStore.getState().appInfo?.version).toBe("9.9.9");

    const emitted = events.emit.mock.calls.map((c) => c[0]);
    expect(emitted).not.toContain(SYNC_EVENT);
    // ...but it does ask the main window for its live state, last.
    expect(emitted[emitted.length - 1]).toBe(SYNC_REQUEST_EVENT);
  });
});

describe("startMainWindowSync", () => {
  it("serves the six forwarded actions with the real store actions", async () => {
    const { startMainWindowSync, useEngineStore, useLibraryStore, useMusicLibraryStore } =
      await load();
    const playPhoneList = vi.fn();
    const playCloudList = vi.fn();
    const syncPhone = vi.fn();
    const invalidateCloud = vi.fn();
    const invalidateYtMusic = vi.fn();
    useEngineStore.setState({ playPhoneList, playCloudList });
    useMusicLibraryStore.setState({ syncPhone, invalidateCloud, invalidateYtMusic });
    const versionBefore = useLibraryStore.getState().version;

    startMainWindowSync();
    await flush();
    const handler = events.handlers.get(MAIN_INVOKE_EVENT);
    expect(handler).toBeDefined();
    const run = (action: string, args: unknown[] = []) =>
      handler?.({ payload: { action, args } });

    run("engine.playPhoneList", ["dev", ["t"], 3]);
    run("engine.playCloudList", [["f"], 0]);
    run("library.refresh");
    run("musicLibrary.syncPhone");
    run("musicLibrary.invalidateCloud");
    run("musicLibrary.invalidateYtMusic");
    run("nope.unknown");

    expect(playPhoneList).toHaveBeenCalledWith("dev", ["t"], 3);
    expect(playCloudList).toHaveBeenCalledWith(["f"], 0);
    expect(useLibraryStore.getState().version).toBe(versionBefore + 1);
    expect(syncPhone).toHaveBeenCalledOnce();
    expect(invalidateCloud).toHaveBeenCalledOnce();
    expect(invalidateYtMusic).toHaveBeenCalledOnce();
  });
});
