/**
 * Product model: HypeMuzik is free forever with every feature unlocked. The
 * only thing sold is the optional one-time "Supporter pack" (pay what you
 * want from $3), which gets the buyer a Supporter badge. Nothing is ever
 * locked behind it.
 *
 * Two independent switches express that:
 */

/**
 * Feature gating. PERMANENTLY `false` — `<PremiumGate>` and `requirePremium()`
 * always pass and no feature ever triggers an upsell. Do not flip this to
 * re-lock the 10 "premium" features; the gate plumbing only remains so the
 * call sites don't need rewriting.
 */
export const FEATURE_GATING_ENABLED = false;

/**
 * Supporter features: the `/me/entitlements` fetch (offline-cached), the
 * "Become a Supporter" sheet (checkout + restore), the Supporter badge and
 * the thank-you toast.
 */
export const SUPPORTER_ENABLED = true;

/** Minimum pay-what-you-want amount, in whole dollars. */
export const SUPPORTER_MIN_AMOUNT = 3;
/** Quick-pick amounts, in whole dollars. */
export const SUPPORTER_QUICK_AMOUNTS = [3, 5, 10, 25] as const;
/** Pre-selected quick pick. */
export const SUPPORTER_DEFAULT_AMOUNT = 5;
