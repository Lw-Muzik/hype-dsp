# Admin full-access + new-user premium trial — design

**Date:** 2026-07-26
**Repos:** `hypemuzik-management` (backend + admin panel), `hypemuzik-desktop`

## Goal
1. The **admin account** is exempt from purchases and always has full access to every premium feature.
2. **New users** get the first *N* days as a premium trial (N = admin-configured), after which premium re-locks to a Supporter purchase. Core playback stays free forever.

## Key facts (already in place)
- `EntitlementsService.evaluate()` is the single source of truth the desktop premium gate consumes (`/me/entitlements` → `{ premium }` → `entitlement.ts` store → `PremiumGate`/`requirePremium`).
- `User.role` (`USER|ADMIN`), `User.trialStartedAt/trialEndsAt`, `User.licensedUntil`, and `AppConfig.defaultTrialDays` (default 14) already exist.
- Signup (`auth.service.ts:62`) and anonymous bootstrap (`:188`) already set `trialEndsAt = now + defaultTrialDays`.
- `GET /me` already returns `role` (via `sanitize(user)` = `Omit<User,'passwordHash'>`).
- `LicenseService.evaluate()` already reports `state` + `daysLeft`.

## Changes

### Management backend
- **`EntitlementsService.evaluate`**: widen the `Pick<User,…>` param to include `role` + `trialEndsAt` (callers pass the full `User`). Return `premium: true` when ANY of:
  - `role === 'ADMIN'` (new — admin exemption, never expires)
  - `licensedUntil > now` (existing)
  - `trialEndsAt > now` (new — trial grants premium)
  - active Supporter on any install (existing)
- **Admin seed**: idempotent script promoting `brunolabs256@gmail.com` to `role='ADMIN'` (only if the user exists).

### Admin panel
- Ensure `defaultTrialDays` is viewable/editable (uses existing `getConfig`/`updateConfig`). Add a field only if missing.

### hype-desktop (UX; features unlock automatically once premium is granted)
- Carry `role` through Rust `UserDto`/`AccountStatus` → TS `AccountStatus`.
- **AccountCard** states:
  - Admin → "Admin · Full access" badge; hide "Become a Supporter" + upsell.
  - Trial (`premium && license.state==='trial'`) → "Premium features · {daysLeft} days left".
  - Free/expired (`!premium`) → existing "Free" + "Become a Supporter".
  - Supporter/licensed → existing "Supporter".
- `openSupporterUpsell()` no-ops when the account is admin.
- Re-lock is automatic: after `trialEndsAt`, `/me/entitlements` returns `premium:false` → store caches false → gate re-locks + upsell returns.

## Testing
- Backend jest: `evaluate` — admin→true, active-trial→true, expired-trial+no-supporter→false, licensed→true, supporter→true.
- Desktop: vitest for AccountCard/entitlement (admin hides purchase, trial shows countdown, expired shows upsell); Rust deser test for `role`.
- Build gates: backend `jest` + build; desktop `cargo test`/`clippy`, `tsc`, `vitest`.

## Out of scope
- No role-toggle UI (admin designated by seed).
- No global trial pill (trial indicator lives in Settings/AccountCard).
- No hard app paywall — only premium features re-lock.
