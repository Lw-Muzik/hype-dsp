import { create } from "zustand";
import {
  ipcErrorMessage,
  onUpdaterStatus,
  updaterRestartNow,
  updaterStatus,
} from "@/lib/ipc";
import type { UpdaterStatus } from "@/lib/ipc";
import { toast } from "@/stores/toast";

interface UpdateStore {
  status: UpdaterStatus;
  /** Version whose banner the user sent to "Later". Session-only on purpose:
   *  the notice must come back on the next launch. */
  dismissedVersion: string | null;
  restarting: boolean;
  /** Subscribe to the backend. Idempotent; safe under StrictMode double-mount. */
  init: () => void;
  dismiss: () => void;
  reopen: () => void;
  restart: () => Promise<void>;
}

let started = false;

export const useUpdateStore = create<UpdateStore>((set, get) => ({
  status: { state: "idle" },
  dismissedVersion: null,
  restarting: false,

  init: () => {
    // The store lives for the whole window, so the listener is never torn down;
    // the flag is what keeps a second mount from stacking another one.
    if (started) return;
    started = true;
    // Subscribe first so an event can't slip between the snapshot and the
    // listener; a late snapshot must not overwrite a newer event, so only apply
    // it if nothing has arrived since.
    let eventSeen = false;
    void onUpdaterStatus((status) => {
      eventSeen = true;
      set({ status });
    }).catch(() => {});
    void updaterStatus()
      .then((status) => {
        if (!eventSeen) set({ status });
      })
      .catch(() => {});
  },

  dismiss: () => {
    const { status } = get();
    if (status.state === "ready") set({ dismissedVersion: status.version });
  },

  reopen: () => set({ dismissedVersion: null }),

  restart: async () => {
    if (get().restarting) return;
    set({ restarting: true });
    try {
      // Never resolves on success — the process is replaced mid-call.
      await updaterRestartNow();
    } catch (e) {
      set({ restarting: false });
      toast.error(ipcErrorMessage(e));
    }
  },
}));

if (import.meta.env.DEV && typeof window !== "undefined") {
  // Lets a dev console fake any updater state to eyeball each UI.
  (window as unknown as { __hmUpdateDemo: (s: UpdaterStatus) => void }).__hmUpdateDemo = (
    status,
  ) => useUpdateStore.setState({ status });
}
