// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * `useEntitlementStore`'s initial `premium` value is read from localStorage
 * synchronously at module-load time (`loadCachedPremium()` inside `create()`),
 * so — like the module-scope IPC calls in `musicLibrary.test.ts` — "what does
 * a fresh launch see" needs a genuinely fresh module instance per scenario.
 * `refresh()`'s runtime behaviour, on the other hand, doesn't depend on
 * import timing at all, so those tests share one store instance and just
 * reset its state + mock between cases (`vi.resetModules()` would otherwise
 * risk the re-imported store and this file's `accountEntitlements` mock
 * resolving to two different `vi.fn()` instances).
 */
vi.mock("@/lib/ipc", () => ({
  accountEntitlements: vi.fn(),
}));

// Payments are currently switched OFF app-wide (`@/config/payments`), which
// short-circuits this store to "always premium, never fetch". Force the flag
// back on so the real entitlement logic stays under test while it's dormant —
// delete when payments are resumed. Off-state is covered by
// `payments-disabled.test.tsx`.
vi.mock("@/config/payments", () => ({ PAYMENTS_ENABLED: true }));

const LS_KEY = "hm.premium";

describe("useEntitlementStore — initial (cached) value", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  async function freshStore() {
    vi.resetModules();
    const { useEntitlementStore } = await import("@/stores/entitlement");
    return useEntitlementStore;
  }

  it("defaults locked when the feature has never been fetched", async () => {
    const useEntitlementStore = await freshStore();
    expect(useEntitlementStore.getState().premium).toBe(false);
  });

  it("boots as a Supporter when the last known value was true", async () => {
    localStorage.setItem(LS_KEY, "1");
    const useEntitlementStore = await freshStore();
    expect(useEntitlementStore.getState().premium).toBe(true);
  });

  it("boots locked when the last known value was explicitly false", async () => {
    localStorage.setItem(LS_KEY, "0");
    const useEntitlementStore = await freshStore();
    expect(useEntitlementStore.getState().premium).toBe(false);
  });

  it("ignores garbage in storage rather than throwing", async () => {
    localStorage.setItem(LS_KEY, "not-a-boolean");
    const useEntitlementStore = await freshStore();
    expect(useEntitlementStore.getState().premium).toBe(false);
  });
});

describe("useEntitlementStore.refresh()", () => {
  // Static import (no resetModules): `refresh()`'s behaviour doesn't depend
  // on module-load timing, so one instance — reset between tests — is both
  // simpler and immune to the mock-identity hazard noted above.
  const storePromise = import("@/stores/entitlement");
  const ipcPromise = import("@/lib/ipc");

  beforeEach(async () => {
    localStorage.clear();
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ premium: false, loading: true });
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockReset();
  });

  it("adopts a genuine server answer and persists it to the cache", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue(true);

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().premium).toBe(true);
    expect(useEntitlementStore.getState().loading).toBe(false);
    expect(localStorage.getItem(LS_KEY)).toBe("1");
  });

  it("downgrades to false only on a genuine (reachable) false answer", async () => {
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ premium: true });
    localStorage.setItem(LS_KEY, "1");
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue(false);

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().premium).toBe(false);
    expect(localStorage.getItem(LS_KEY)).toBe("0");
  });

  it("keeps a cached Supporter's premium when the check fails offline", async () => {
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ premium: true });
    localStorage.setItem(LS_KEY, "1");
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockRejectedValue(new Error("couldn't reach the server"));

    await useEntitlementStore.getState().refresh();

    // A blip must never downgrade a supporter mid-session (or on a first,
    // offline launch after a previous online one).
    expect(useEntitlementStore.getState().premium).toBe(true);
    expect(useEntitlementStore.getState().loading).toBe(false);
    expect(localStorage.getItem(LS_KEY)).toBe("1");
  });

  it("stays locked (never fabricates true) when a never-fetched account is offline", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockRejectedValue(new Error("couldn't reach the server"));

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().premium).toBe(false);
    expect(useEntitlementStore.getState().loading).toBe(false);
  });

  it("stops loading after the very first check, success or failure", async () => {
    const { useEntitlementStore } = await storePromise;
    expect(useEntitlementStore.getState().loading).toBe(true);
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue(false);

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().loading).toBe(false);
  });
});
