// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Initial `premium`/`supporter` values are read from localStorage
 * synchronously at module-load time, so "what does a fresh launch see" needs a
 * genuinely fresh module instance per scenario. `refresh()`'s runtime
 * behaviour doesn't depend on import timing, so those tests share one store
 * instance and reset its state + mock between cases.
 */
vi.mock("@/lib/ipc", () => ({
  accountEntitlements: vi.fn(),
}));

const LS_PREMIUM = "hm.premium";
const LS_SUPPORTER = "hm.supporter";

describe("useEntitlementStore — initial (cached) values", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  async function freshStore() {
    vi.resetModules();
    const { useEntitlementStore } = await import("@/stores/entitlement");
    return useEntitlementStore;
  }

  it("defaults to not-a-supporter when nothing was ever fetched", async () => {
    const s = (await freshStore()).getState();
    expect(s.premium).toBe(false);
    expect(s.supporter).toBe(false);
  });

  it("boots as a Supporter when the last known value was true", async () => {
    localStorage.setItem(LS_SUPPORTER, "1");
    const s = (await freshStore()).getState();
    expect(s.supporter).toBe(true);
    expect(s.premium).toBe(false);
  });

  it("caches premium and supporter independently", async () => {
    localStorage.setItem(LS_PREMIUM, "1");
    localStorage.setItem(LS_SUPPORTER, "0");
    const s = (await freshStore()).getState();
    expect(s.premium).toBe(true);
    expect(s.supporter).toBe(false);
  });

  it("ignores garbage in storage rather than throwing", async () => {
    localStorage.setItem(LS_SUPPORTER, "not-a-boolean");
    expect((await freshStore()).getState().supporter).toBe(false);
  });
});

describe("useEntitlementStore.refresh()", () => {
  const storePromise = import("@/stores/entitlement");
  const ipcPromise = import("@/lib/ipc");

  beforeEach(async () => {
    localStorage.clear();
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ premium: false, supporter: false, loading: true });
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockReset();
  });

  it("adopts a genuine server answer and persists both flags", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue({ premium: true, supporter: true });

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().supporter).toBe(true);
    expect(useEntitlementStore.getState().premium).toBe(true);
    expect(useEntitlementStore.getState().loading).toBe(false);
    expect(localStorage.getItem(LS_SUPPORTER)).toBe("1");
    expect(localStorage.getItem(LS_PREMIUM)).toBe("1");
  });

  it("premium without supporter (admin/trial) does NOT make a supporter", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue({ premium: true, supporter: false });

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().premium).toBe(true);
    expect(useEntitlementStore.getState().supporter).toBe(false);
    expect(localStorage.getItem(LS_SUPPORTER)).toBe("0");
  });

  it("treats a missing supporter field (older backend) as false", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    useEntitlementStore.setState({ supporter: true });
    vi.mocked(ipc.accountEntitlements).mockResolvedValue({ premium: true } as never);

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().supporter).toBe(false);
    expect(localStorage.getItem(LS_SUPPORTER)).toBe("0");
  });

  it("downgrades only on a genuine (reachable) false answer", async () => {
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ supporter: true });
    localStorage.setItem(LS_SUPPORTER, "1");
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue({ premium: false, supporter: false });

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().supporter).toBe(false);
    expect(localStorage.getItem(LS_SUPPORTER)).toBe("0");
  });

  it("keeps a cached Supporter's badge when the check fails offline", async () => {
    const { useEntitlementStore } = await storePromise;
    useEntitlementStore.setState({ supporter: true });
    localStorage.setItem(LS_SUPPORTER, "1");
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockRejectedValue(new Error("couldn't reach the server"));

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().supporter).toBe(true);
    expect(useEntitlementStore.getState().loading).toBe(false);
    expect(localStorage.getItem(LS_SUPPORTER)).toBe("1");
  });

  it("never fabricates a supporter when a never-fetched account is offline", async () => {
    const { useEntitlementStore } = await storePromise;
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockRejectedValue(new Error("couldn't reach the server"));

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().supporter).toBe(false);
    expect(useEntitlementStore.getState().loading).toBe(false);
  });

  it("stops loading after the very first check, success or failure", async () => {
    const { useEntitlementStore } = await storePromise;
    expect(useEntitlementStore.getState().loading).toBe(true);
    const ipc = await ipcPromise;
    vi.mocked(ipc.accountEntitlements).mockResolvedValue({ premium: false, supporter: false });

    await useEntitlementStore.getState().refresh();

    expect(useEntitlementStore.getState().loading).toBe(false);
  });
});
