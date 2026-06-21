import { create } from "zustand";
import type { AccountStatus, LicenseStatus } from "@/lib/types";
import {
  accountLogout,
  accountRequestOtp,
  accountSignup,
  accountStatus,
  accountVerify,
} from "@/lib/ipc";

interface AccountStore {
  status: AccountStatus | null;
  loading: boolean;
  /** Re-fetch the account + license from the server. */
  refresh: () => Promise<void>;
  /** Create an account (email + optional name); the server emails a code. */
  signup: (email: string, name?: string) => Promise<void>;
  /** Request a sign-in code for an existing account. */
  requestOtp: (email: string) => Promise<void>;
  /** Verify the emailed code → starts the session. */
  verify: (email: string, code: string) => Promise<void>;
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
  signup: (email, name) => accountSignup(email, name),
  requestOtp: (email) => accountRequestOtp(email),
  verify: async (email, code) => {
    const status = await accountVerify(email, code);
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
