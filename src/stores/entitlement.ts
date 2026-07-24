import { create } from "zustand";
import { accountEntitlements } from "@/lib/ipc";

/**
 * Supporter entitlement — the single source of truth the premium gate
 * consumes (`usePremium`, `PremiumGate`, `requirePremium`).
 *
 * Unlike the account store, this is deliberately **cached**: a Supporter who
 * opens the app offline must keep every premium feature working, so the last
 * server answer is persisted to `localStorage` and used until a fresh one
 * arrives. A failed/offline check never downgrades it — only a genuine `200`
 * response (true *or* false) updates the cache. A user who has never
 * successfully checked defaults to locked (`false`), matching "Advanced DSP,
 * stems, and cloud require Supporter" being the safe default.
 */

const LS_KEY = "hm.premium";

function loadCachedPremium(): boolean {
  try {
    return localStorage.getItem(LS_KEY) === "1";
  } catch {
    return false;
  }
}

function saveCachedPremium(premium: boolean): void {
  try {
    localStorage.setItem(LS_KEY, premium ? "1" : "0");
  } catch {
    // Private mode / no storage — just won't persist across relaunches.
  }
}

/** Gated feature keys, grouped as in the spec: Advanced DSP suite, stem
 *  separation, and cloud music. Value is the human label for upsell copy. */
export const PREMIUM_FEATURES = {
  SYSTEM_EQ: "System-wide EQ",
  SURROUND_3D: "3D Surround",
  ROOM_REVERB: "Room reverb",
  CONVOLVER: "Convolver",
  AUTOEQ: "AutoEQ",
  VIPER_DDC: "ViPER DDC curves",
  PER_APP_EQ: "Per-app EQ",
  PER_APP_MIXER: "Per-app mixer",
  STEMS: "Stem separation",
  CLOUD: "Cloud music",
} as const;

export type PremiumFeature = keyof typeof PREMIUM_FEATURES;

interface EntitlementStore {
  premium: boolean;
  /** True only until the very first check of this session settles (success or
   *  failure) — a brief "checking…" window, not a persistent spinner. */
  loading: boolean;
  /** Fetch the real entitlement and refresh the cache; a failure (offline,
   *  no session yet, server error) is swallowed and leaves `premium` as-is. */
  refresh: () => Promise<void>;
}

export const useEntitlementStore = create<EntitlementStore>((set) => ({
  premium: loadCachedPremium(),
  loading: true,
  refresh: async () => {
    try {
      const premium = await accountEntitlements();
      saveCachedPremium(premium);
      set({ premium, loading: false });
    } catch {
      set({ loading: false });
    }
  },
}));

/** Whether the account can use Supporter-gated features right now (cached
 *  offline-safe). Use inside components; for imperative checks in event
 *  handlers, read `useEntitlementStore.getState().premium` instead. */
export function usePremium(): boolean {
  return useEntitlementStore((s) => s.premium);
}
