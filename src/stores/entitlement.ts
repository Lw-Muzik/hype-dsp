import { create } from "zustand";
import { accountEntitlements } from "@/lib/ipc";
import { SUPPORTER_ENABLED } from "@/config/payments";

/**
 * Entitlement store. Two independent facts from `GET /me/entitlements`:
 *
 *  - `supporter` — the account owns a HypeMuzik Plus purchase (the backend
 *    stores it as a Supporter row). Drives the badge and the "You have Plus"
 *    state, and counts as entitled.
 *  - `premium` — also true for admin/trial/licensed accounts. Entitles the
 *    Plus features but must NOT drive the badge.
 *
 * Both are **cached**: a Plus owner who opens the app offline keeps their
 * features, so the last server answer is persisted to `localStorage` and used
 * until a fresh one arrives. A failed/offline check never downgrades either
 * flag — only a genuine `200` response (true *or* false) updates the cache.
 * A backend that predates `supporter` reports it as false.
 *
 * Before the first-ever answer (no cache, nothing fetched) both flags are
 * false, which is indistinguishable from "not entitled" — `usePlusEnforcement`
 * therefore only ever *suppresses* Plus effects reversibly.
 */

const LS_PREMIUM = "hm.premium";
const LS_SUPPORTER = "hm.supporter";
const LS_TRIAL_ENDS = "hm.trialEndsAt";

function loadCached(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function loadCachedTrial(): string | null {
  try {
    return localStorage.getItem(LS_TRIAL_ENDS) || null;
  } catch {
    return null;
  }
}

function saveCachedTrial(value: string | null): void {
  try {
    if (value) localStorage.setItem(LS_TRIAL_ENDS, value);
    else localStorage.removeItem(LS_TRIAL_ENDS);
  } catch {
    // Private mode / no storage — just won't persist across relaunches.
  }
}

/** Whole days left until `trialEndsAt`, rounded up; 0 on the last day (ends
 *  within 24 h); `null` when there is no trial, it is unparseable, or it has
 *  already ended. */
export function trialDaysLeft(trialEndsAt: string | null | undefined, now = Date.now()): number | null {
  if (!trialEndsAt) return null;
  const end = Date.parse(trialEndsAt);
  if (!Number.isFinite(end) || end <= now) return null;
  return Math.ceil((end - now) / 86_400_000);
}

/** True when the trial's last moment falls on today's (local) calendar day. */
export function trialEndsToday(trialEndsAt: string | null | undefined, now = Date.now()): boolean {
  if (trialDaysLeft(trialEndsAt, now) === null) return false;
  return new Date(Date.parse(trialEndsAt as string)).toDateString() === new Date(now).toDateString();
}

const LS_TRIAL_NOTICE = "hm.trialEndedNoticeFor";

/** Whether the one-time "your trial has ended" upsell text was already shown
 *  for this trial. Pure read; pair with {@link markTrialEndedNoticeSeen}. */
export function trialEndedNoticeSeen(trialEndsAt: string | null): boolean {
  try {
    return localStorage.getItem(LS_TRIAL_NOTICE) === trialEndsAt;
  } catch {
    return false;
  }
}

export function markTrialEndedNoticeSeen(trialEndsAt: string | null): void {
  try {
    if (trialEndsAt) localStorage.setItem(LS_TRIAL_NOTICE, trialEndsAt);
  } catch {
    // No storage: the notice may show again, harmless.
  }
}

/** True when a trial end date exists and is in the past. */
export function trialHasEnded(trialEndsAt: string | null | undefined, now = Date.now()): boolean {
  if (!trialEndsAt) return false;
  const end = Date.parse(trialEndsAt);
  return Number.isFinite(end) && end <= now;
}

function saveCached(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    // Private mode / no storage — just won't persist across relaunches.
  }
}

/** Labels for every feature key; which ones Plus locks is `PLUS_FEATURES`. */
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
  /** Owns a HypeMuzik Plus purchase — the only thing the badge reads. */
  supporter: boolean;
  /** End of the free Plus trial (ISO 8601), or null (none / older backend). */
  trialEndsAt: string | null;
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
  trialEndsAt: SUPPORTER_ENABLED ? loadCachedTrial() : null,
  loading: SUPPORTER_ENABLED,
  refresh: async () => {
    if (!SUPPORTER_ENABLED) return;
    try {
      const e = await accountEntitlements();
      const premium = !!e.premium;
      // Missing/older-backend `supporter` is false.
      const supporter = e.supporter === true;
      saveCached(LS_PREMIUM, premium);
      // Missing field (older backend) is null.
      const trialEndsAt = typeof e.trialEndsAt === "string" && e.trialEndsAt ? e.trialEndsAt : null;
      saveCached(LS_SUPPORTER, supporter);
      saveCachedTrial(trialEndsAt);
      set({ premium, supporter, trialEndsAt, loading: false });
    } catch {
      set({ loading: false });
    }
  },
}));

/** Whether the account owns a Plus purchase (cached, offline-safe). */
export function useSupporter(): boolean {
  return useEntitlementStore((s) => s.supporter);
}
