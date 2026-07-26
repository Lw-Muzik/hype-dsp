import { create } from "zustand";
import type { AccountStatus } from "@/lib/types";
import {
  accountBootstrapAnonymous,
  accountClaimRequest,
  accountClaimVerify,
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
  /** Silently provision the frictionless anonymous session (no email, no
   *  password) when there's no local session yet. Never throws — offline just
   *  leaves `status` as it was (core playback needs no account). */
  bootstrapAnonymous: () => Promise<void>;
  /** Create an account (email + optional name); the server emails a code. */
  signup: (email: string, name?: string) => Promise<void>;
  /** Request a sign-in code for an existing account. */
  requestOtp: (email: string) => Promise<void>;
  /** Verify the emailed code → starts the session. */
  verify: (email: string, code: string) => Promise<void>;
  /** Start attaching a real email to the current account (Settings). */
  claimRequest: (email: string) => Promise<void>;
  /** Verify the emailed code and attach it to the current account in place. */
  claimVerify: (email: string, code: string, name?: string) => Promise<void>;
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
  bootstrapAnonymous: async () => {
    try {
      const status = await accountBootstrapAnonymous();
      set({ status, loading: false });
    } catch {
      // Offline on first launch (or between sessions) — core is free and
      // needs no account, so just stop the spinner; the next focus/interval
      // refresh in SessionGate retries.
      set({ loading: false });
    }
  },
  signup: (email, name) => accountSignup(email, name),
  requestOtp: (email) => accountRequestOtp(email),
  verify: async (email, code) => {
    const status = await accountVerify(email, code);
    set({ status });
  },
  claimRequest: (email) => accountClaimRequest(email),
  claimVerify: async (email, code, name) => {
    const status = await accountClaimVerify(email, code, name);
    set({ status });
  },
  logout: async () => {
    await accountLogout().catch(() => {});
    set({
      status: {
        authenticated: false,
        email: null,
        name: null,
        license: null,
        emailVerified: false,
        isAdmin: false,
      },
    });
  },
}));
