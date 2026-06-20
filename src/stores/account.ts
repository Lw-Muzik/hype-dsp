import { create } from "zustand";
import type { AccountStatus, LicenseStatus } from "@/lib/types";
import {
  accountLogin,
  accountLogout,
  accountSignup,
  accountStatus,
} from "@/lib/ipc";

interface AccountStore {
  status: AccountStatus | null;
  loading: boolean;
  /** Re-fetch the account + license from the server. */
  refresh: () => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  signup: (email: string, password: string, name?: string) => Promise<void>;
  logout: () => Promise<void>;
}

export const useAccountStore = create<AccountStore>((set) => ({
  status: null,
  loading: true,
  refresh: async () => {
    try {
      const status = await accountStatus();
      set({ status, loading: false });
    } catch {
      set({ loading: false });
    }
  },
  login: async (email, password) => {
    const status = await accountLogin(email, password);
    set({ status });
  },
  signup: async (email, password, name) => {
    const status = await accountSignup(email, password, name);
    set({ status });
  },
  logout: async () => {
    await accountLogout().catch(() => {});
    set({
      status: { authenticated: false, email: null, name: null, license: null },
    });
  },
}));

/** Map the server license onto the UI's existing LicenseStatus (TrialBanner). */
export function toLicenseStatus(status: AccountStatus | null): LicenseStatus | null {
  const license = status?.license;
  if (!license) return null;
  if (license.state === "licensed") return { kind: "licensed" };
  if (license.state === "trial") return { kind: "trial", daysLeft: license.daysLeft };
  return { kind: "expired" };
}
