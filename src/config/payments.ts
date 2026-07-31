/**
 * Master kill switch for everything monetisation-related.
 *
 * While this is `false` the app behaves as fully free software: every
 * Supporter-gated feature is unlocked, no checkout/restore UI is reachable,
 * and no entitlement request is made to the backend. The Paddle checkout
 * flow, the restore-by-email flow and the entitlement store all still exist
 * in the tree — they are simply unreachable — so payments can be resumed
 * without rewriting anything.
 *
 * TO RESUME PAYMENTS:
 *   1. Flip this to `true`.
 *   2. Delete the `vi.mock("@/config/payments", ...)` line at the top of
 *      `PremiumGate.test.tsx`, `entitlement.test.ts` and `AccountCard.test.tsx`
 *      (they force it back on so the payment logic stays under test while it
 *      is switched off), and drop `payments-disabled.test.tsx`.
 *
 * Turning this back on restores, with no other change:
 *   - `<PremiumGate>` / `requirePremium()` gating of the 10 premium features
 *   - the Supporter upsell + Paddle checkout modal and the unlock toast
 *   - the `/me/entitlements` check and its offline-safe cache
 *   - the Settings → Account plan badge, trial note and "Become a Supporter" CTA
 *
 * Note: the backend (`hypemuzik-management`) still serves `/support/checkout`,
 * `/support/restore` and `/webhooks/paddle`. This switch only stops the
 * desktop app from calling them.
 */
export const PAYMENTS_ENABLED = false;
