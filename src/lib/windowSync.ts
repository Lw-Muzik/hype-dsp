import { emit, emitTo, listen } from "@tauri-apps/api/event";
import type { StoreApi } from "zustand";
import { MAIN_WINDOW, WINDOW_LABEL } from "@/lib/windowLabel";

/**
 * Keeps chosen Zustand state identical in every app window.
 *
 * Each Tauri window is its own JS context with its own store instances, so the
 * Settings window and the main window would drift apart: a theme picked in
 * Settings wouldn't reach the main window until a relaunch, and a Settings
 * window opened with default engine state would send those defaults back to
 * Rust (`setPlayback` sends gapless and crossfade together). So:
 *
 * - a change to a synced key is emitted as just the changed keys, stamped with
 *   the sender's label; other windows apply it without re-emitting;
 * - a window that opens asks the others for a snapshot, so it starts from their
 *   live state rather than whatever localStorage held when its modules loaded;
 * - persistence stays where it was: the window where the user acted writes its
 *   localStorage keys, receivers don't write again.
 *
 * Actions that drive the main window's player or library (start a queue,
 * re-sync the phone) must run there, not be mirrored: see {@link forwardToMain}.
 */

export const SYNC_EVENT = "hm://sync";
export const SYNC_REQUEST_EVENT = "hm://sync-request";
export const MAIN_INVOKE_EVENT = "hm://main-invoke";

type Patch = Record<string, unknown>;

export interface SyncMessage {
  store: string;
  from: string;
  /** A full snapshot answering a window that just opened, not a change. */
  snapshot?: boolean;
  patch: Patch;
}

export interface SyncSpec<S> {
  /** Top-level keys mirrored whole. Compared by identity, as Zustand sets do. */
  keys?: readonly (keyof S & string)[];
  /**
   * Object-valued keys mirrored field by field: only the listed fields are
   * compared and merged, so one window can't overwrite fields another window
   * changed in the meantime (the engine's `state`).
   */
  merge?: { readonly [K in keyof S & string]?: readonly string[] };
}

type Obj = Record<string, unknown>;

const asObj = (v: unknown): Obj => (v && typeof v === "object" ? (v as Obj) : {});

/** The synced keys that differ between `prev` and `next`, or null if none. */
export function diffSynced<S extends object>(prev: S, next: S, spec: SyncSpec<S>): Patch | null {
  const patch: Patch = {};
  const p = prev as Obj;
  const n = next as Obj;
  for (const key of spec.keys ?? []) {
    if (!Object.is(p[key], n[key])) patch[key] = n[key];
  }
  for (const [key, fields] of Object.entries(spec.merge ?? {}) as [string, readonly string[]][]) {
    if (Object.is(p[key], n[key])) continue;
    const before = asObj(p[key]);
    const after = asObj(n[key]);
    const changed: Obj = {};
    for (const f of fields) {
      if (!Object.is(before[f], after[f])) changed[f] = after[f];
    }
    if (Object.keys(changed).length) patch[key] = changed;
  }
  return Object.keys(patch).length ? patch : null;
}

/** Every synced key of `state`, for a snapshot. */
export function snapshotSynced<S extends object>(state: S, spec: SyncSpec<S>): Patch {
  const s = state as Obj;
  const patch: Patch = {};
  for (const key of spec.keys ?? []) patch[key] = s[key];
  for (const [key, fields] of Object.entries(spec.merge ?? {}) as [string, readonly string[]][]) {
    const value = asObj(s[key]);
    const picked: Obj = {};
    for (const f of fields) picked[f] = value[f];
    patch[key] = picked;
  }
  return patch;
}

/** The partial state that applies `patch` to `state` (merge keys merged in). */
export function applySynced<S extends object>(state: S, patch: Patch, spec: SyncSpec<S>): Partial<S> {
  const s = state as Obj;
  const merge = (spec.merge ?? {}) as Record<string, readonly string[] | undefined>;
  const out: Obj = {};
  for (const [key, value] of Object.entries(patch)) {
    out[key] = merge[key] ? { ...asObj(s[key]), ...asObj(value) } : value;
  }
  return out as Partial<S>;
}

/** Patch paths (`key` or `key.field`), to remember what changed locally. */
function paths(patch: Patch, spec: { merge?: object }): string[] {
  const merge = (spec.merge ?? {}) as Record<string, unknown>;
  return Object.entries(patch).flatMap(([key, value]) =>
    merge[key] ? Object.keys(asObj(value)).map((f) => `${key}.${f}`) : [key],
  );
}

/** `patch` without the paths in `skip`; null if nothing is left. */
function without(patch: Patch, skip: ReadonlySet<string>, spec: { merge?: object }): Patch | null {
  const merge = (spec.merge ?? {}) as Record<string, unknown>;
  const out: Patch = {};
  for (const [key, value] of Object.entries(patch)) {
    if (merge[key]) {
      const kept = Object.fromEntries(
        Object.entries(asObj(value)).filter(([f]) => !skip.has(`${key}.${f}`)),
      );
      if (Object.keys(kept).length) out[key] = kept;
    } else if (!skip.has(key)) {
      out[key] = value;
    }
  }
  return Object.keys(out).length ? out : null;
}

/** Set once this window has asked the others for a snapshot. */
let awaitingSnapshot = false;

/**
 * Mirror `spec`'s keys of `store` across windows under `name`. Resolves once
 * the listeners are registered (call {@link requestSnapshot} after that).
 * Returns a function that stops syncing.
 */
export async function syncStore<S extends object>(
  store: StoreApi<S>,
  name: string,
  spec: SyncSpec<S>,
): Promise<() => void> {
  // Set while applying another window's change, so it isn't sent back.
  let applying = false;
  let snapshotApplied = false;
  // Paths the user changed here before the snapshot arrived; the snapshot must
  // not roll them back.
  const changedEarly = new Set<string>();

  const unsubscribe = store.subscribe((next, prev) => {
    if (applying) return;
    const patch = diffSynced(prev, next, spec);
    if (!patch) return;
    if (awaitingSnapshot && !snapshotApplied) paths(patch, spec).forEach((p) => changedEarly.add(p));
    const message: SyncMessage = { store: name, from: WINDOW_LABEL, patch };
    void emit(SYNC_EVENT, message).catch(() => {});
  });

  const apply = (patch: Patch) => {
    applying = true;
    try {
      store.setState((state) => applySynced(state, patch, spec));
    } finally {
      applying = false;
    }
  };

  const unlistenSync = await listen<SyncMessage>(SYNC_EVENT, ({ payload }) => {
    if (payload.store !== name || payload.from === WINDOW_LABEL) return;
    if (payload.snapshot) {
      if (snapshotApplied) return;
      snapshotApplied = true;
      const patch = without(payload.patch, changedEarly, spec);
      if (patch) apply(patch);
      return;
    }
    apply(payload.patch);
  });

  const unlistenRequest = await listen<{ from: string }>(SYNC_REQUEST_EVENT, ({ payload }) => {
    if (payload.from === WINDOW_LABEL) return;
    const message: SyncMessage = {
      store: name,
      from: WINDOW_LABEL,
      snapshot: true,
      patch: snapshotSynced(store.getState(), spec),
    };
    void emitTo(payload.from, SYNC_EVENT, message).catch(() => {});
  });

  return () => {
    unsubscribe();
    unlistenSync();
    unlistenRequest();
  };
}

/**
 * Ask the other windows for their synced state. Call once, after every
 * {@link syncStore} in this window has resolved.
 */
export async function requestSnapshot(): Promise<void> {
  awaitingSnapshot = true;
  await emit(SYNC_REQUEST_EVENT, { from: WINDOW_LABEL });
}

interface MainInvocation {
  action: string;
  args: unknown[];
}

/**
 * Run `action` in the main window. For actions whose effects live in the main
 * window's stores (starting a queue that the now-playing bar shows, re-syncing
 * the phone library the Library view renders): mirroring their results would
 * be both incomplete and racy, so the main window does the work itself.
 */
export function forwardToMain(action: string, args: unknown[]): void {
  const message: MainInvocation = { action, args };
  void emitTo(MAIN_WINDOW, MAIN_INVOKE_EVENT, message).catch(() => {});
}

/** In the main window: carry out actions other windows forward. */
export function serveMainInvocations(
  handlers: Readonly<Record<string, (...args: never[]) => unknown>>,
): Promise<() => void> {
  return listen<MainInvocation>(MAIN_INVOKE_EVENT, ({ payload }) => {
    const handler = handlers[payload.action] as ((...args: unknown[]) => unknown) | undefined;
    if (!handler) return;
    try {
      void Promise.resolve(handler(...payload.args)).catch(() => {});
    } catch {
      // A forwarded action failing must not take the listener down.
    }
  });
}

/** Test-only: forget a pending snapshot request. */
export function resetWindowSyncForTests(): void {
  awaitingSnapshot = false;
}
