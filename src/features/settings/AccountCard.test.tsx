// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AccountCard } from "@/features/settings/AccountCard";
import { useAccountStore } from "@/stores/account";
import { useEntitlementStore } from "@/stores/entitlement";
import type { AccountStatus, LicenseInfo } from "@/lib/types";

// Payments are currently switched OFF app-wide (`@/config/payments`), which
// hides the Plan row and the Supporter CTA. Force the flag back on so the
// plan/badge/upsell logic stays under test while it's dormant — delete when
// payments are resumed. Off-state is covered by `payments-disabled.test.tsx`.
vi.mock("@/config/payments", () => ({ PAYMENTS_ENABLED: true }));

// Keep the pure `ipcErrorMessage` real; stub only the network-hitting calls so
// nothing reaches Tauri during a render-only test.
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    accountSupportCheckout: vi.fn(),
    accountSupportRestore: vi.fn(),
    openExternalUrl: vi.fn(),
  };
});

const license = (over: Partial<LicenseInfo> = {}): LicenseInfo => ({
  state: "expired",
  allowed: true,
  daysLeft: 0,
  trialEndsAt: null,
  licensedUntil: null,
  ...over,
});

const status = (over: Partial<AccountStatus> = {}): AccountStatus => ({
  authenticated: true,
  email: "user@example.com",
  name: "User",
  license: license(),
  emailVerified: true,
  isAdmin: false,
  ...over,
});

/** Render the card with a given account status + entitlement premium flag. */
function mount(account: AccountStatus, premium: boolean) {
  useAccountStore.setState({ status: account });
  useEntitlementStore.setState({
    premium,
    loading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
  });
  return render(<AccountCard />);
}

const upsellButton = () =>
  screen.queryByRole("button", { name: /Become a Supporter/i });

beforeEach(() => {
  useAccountStore.setState({ status: null });
  useEntitlementStore.setState({
    premium: false,
    loading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
  });
});

afterEach(() => cleanup());

describe("<AccountCard> plan states", () => {
  it("admin: shows an Admin badge, hides the purchase CTA, notes full access", () => {
    // Even with entitlement not-premium, the admin is exempt + full access.
    mount(status({ isAdmin: true, license: license({ state: "licensed" }) }), false);

    expect(screen.getByText("Admin")).toBeInTheDocument();
    expect(upsellButton()).not.toBeInTheDocument();
    expect(screen.getByText(/Full access to every feature/i)).toBeInTheDocument();
    expect(screen.queryByText("Free")).not.toBeInTheDocument();
    expect(screen.queryByText("Supporter")).not.toBeInTheDocument();
  });

  it("trial: shows a countdown badge + trial note, and still offers the CTA to convert early", () => {
    mount(status({ license: license({ state: "trial", daysLeft: 7 }) }), true);

    expect(screen.getByText("Trial · 7 days left")).toBeInTheDocument();
    expect(screen.getByText(/free trial of the premium suite/i)).toBeInTheDocument();
    // Trial users can still convert early.
    expect(upsellButton()).toBeInTheDocument();
  });

  it("trial: singularises the last day", () => {
    mount(status({ license: license({ state: "trial", daysLeft: 1 }) }), true);
    expect(screen.getByText("Trial · 1 day left")).toBeInTheDocument();
  });

  it("free / expired: shows Free and the purchase CTA", () => {
    mount(status({ license: license({ state: "expired", daysLeft: 0 }) }), false);

    expect(screen.getByText("Free")).toBeInTheDocument();
    expect(upsellButton()).toBeInTheDocument();
    expect(screen.queryByText(/free trial of the premium suite/i)).not.toBeInTheDocument();
  });

  it("supporter: shows Supporter and hides the purchase CTA", () => {
    // Premium via purchase/license, not a trial.
    mount(status({ license: license({ state: "licensed" }) }), true);

    expect(screen.getByText("Supporter")).toBeInTheDocument();
    expect(upsellButton()).not.toBeInTheDocument();
  });
});
