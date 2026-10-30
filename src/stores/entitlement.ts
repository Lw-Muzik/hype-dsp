import { create } from "zustand";
import { accountEntitlements } from "@/lib/ipc";
import { SUPPORTER_ENABLED } from "@/config/payments";

/**
 * Entitlement store. Two independent facts from `GET /me/entitlements`:
 *
 *  - `supporter` — the account owns the (optional, one-time) Supporter pack.
 *    It drives the Supporter badge and nothing else; no feature is gated on it.
 *  - `premium` — also true for admin/trial/licensed accounts. Kept only for
 *    the (permanently off) feature-gate plumbing; it must NOT drive the badge.
 *
 * Both are **cached**: a Supporter who opens the app offline keeps their
 * badge, so the last server answer is persisted to `localStorage` and used
 * until a fresh one arrives. A failed/offline check never downgrades either
 * flag — only a genuine `200` response (true *or* false) updates the cache.
 * A backend that predates `supporter` reports it as false.
 */

const LS_PREMIUM = "hm.premium";
const LS_SUPPORTER = "hm.supporter";

function loadCached(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function saveCached(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    // Private mode / no storage — just won't persist across relaunches.
  }
}

/** Labels for the (ungated) feature keys, kept for the gate plumbing. */
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
  /** Owns the Supporter pack — the only thing the badge reads. */
  supporter: boolean;
  /** True only until the very first check of this session settles (success or
   *  failure) — a brief "checking…" window, not a persistent spinner. */
  loading: boolean;
  /** Fetch the real entitlements and refresh the cache; a failure (offline,
   *  no session yet, server error) is swallowed and leaves flags as-is. */
  refresh: () => Promise<void>;
}

export const useEntitlementStore = create<EntitlementStore>((set) => ({
  premium: SUPPORTER_ENABLED ? loadCached(LS_PREMIUM) : false,
  supporter: SUPPORTER_ENABLED ? loadCached(LS_SUPPORTER) : false,
  loading: SUPPORTER_ENABLED,
  refresh: async () => {
    if (!SUPPORTER_ENABLED) return;
    try {
      const e = await accountEntitlements();
      const premium = !!e.premium;
      // Missing/older-backend `supporter` is false.
      const supporter = e.supporter === true;
      saveCached(LS_PREMIUM, premium);
      saveCached(LS_SUPPORTER, supporter);
      set({ premium, supporter, loading: false });
    } catch {
      set({ loading: false });
    }
  },
}));

/** Whether the account owns the Supporter pack (cached, offline-safe). */
export function useSupporter(): boolean {
  return useEntitlementStore((s) => s.supporter);
}
