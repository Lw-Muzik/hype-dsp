import type { PremiumFeature } from "@/stores/entitlement";

/**
 * Product model: HypeMuzik is free, and the optional one-time "HypeMuzik Plus"
 * licence (US$5, lifetime, no subscription) unlocks six advanced features.
 * Everything else is free forever.
 *
 * Two independent switches express that:
 */

/**
 * The features locked behind HypeMuzik Plus. `<PremiumGate>` and
 * `requirePremium()` pass immediately for any feature NOT in this set (system
 * EQ, AutoEQ, stems, cloud, ...); for a feature in it they pass only when the
 * account is entitled (backend `premium`, `supporter`, or a local admin).
 */
export const PLUS_FEATURES: ReadonlySet<PremiumFeature> = new Set<PremiumFeature>([
  "PER_APP_EQ",
  "PER_APP_MIXER",
  "CONVOLVER",
  "SURROUND_3D",
  "ROOM_REVERB",
  "VIPER_DDC",
]);

/**
 * Plus support: the `/me/entitlements` fetch (offline-cached), the "HypeMuzik
 * Plus" sheet (checkout + restore), the badge and the thank-you toast.
 */
export const SUPPORTER_ENABLED = true;
