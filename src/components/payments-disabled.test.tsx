// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

/**
 * The switched-off half of the payment kill switch (`@/config/payments`).
 *
 * Every other payment suite mocks `PAYMENTS_ENABLED` back to `true` so the
 * dormant checkout/gating logic stays covered; this file is the one place
 * that asserts what actually ships today — nothing is gated, no purchase UI
 * exists, and no entitlement request is made. It deliberately does NOT mock
 * the flag, so if someone flips it back on without revisiting these
 * expectations, this suite fails loudly instead of silently passing.
 */
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    accountEntitlements: vi.fn().mockResolvedValue(true),
    accountSupportCheckout: vi.fn(),
    accountSupportRestore: vi.fn(),
    openExternalUrl: vi.fn(),
  };
});

import {
  openSupporterUpsell,
  PremiumGate,
  requirePremium,
  SupporterUpsellModal,
} from "@/components/PremiumGate";
import { PREMIUM_FEATURES, useEntitlementStore } from "@/stores/entitlement";
import { AccountCard } from "@/features/settings/AccountCard";
import { useAccountStore } from "@/stores/account";
import { accountEntitlements } from "@/lib/ipc";
import { PAYMENTS_ENABLED } from "@/config/payments";
import type { AccountStatus } from "@/lib/types";

/**
 * The entitlement store's initial state, captured at module load — before any
 * test below mutates it. `set()` replaces the state object rather than
 * mutating it, so this reference stays a true snapshot of what the
 * initializer produced on a fresh launch.
 */
const INITIAL_ENTITLEMENT = useEntitlementStore.getState();

const FREE_STATUS: AccountStatus = {
  authenticated: true,
  email: null,
  name: null,
  license: null,
  emailVerified: false,
  isAdmin: false,
};

beforeEach(() => {
  useAccountStore.setState({ status: FREE_STATUS });
  vi.mocked(accountEntitlements).mockClear();
});

afterEach(() => {
  cleanup();
});

describe("payments kill switch", () => {
  it("is off — payments are disabled for this build", () => {
    expect(PAYMENTS_ENABLED).toBe(false);
  });
});

describe("gating with payments disabled", () => {
  it("renders children for every gated feature instead of an upsell", () => {
    for (const feature of Object.keys(PREMIUM_FEATURES) as Array<
      keyof typeof PREMIUM_FEATURES
    >) {
      const { unmount } = render(
        <PremiumGate feature={feature}>
          <div>{`unlocked:${feature}`}</div>
        </PremiumGate>,
      );
      expect(screen.getByText(`unlocked:${feature}`)).toBeInTheDocument();
      expect(screen.queryByText("Become a Supporter")).not.toBeInTheDocument();
      unmount();
    }
  });

  it("lets every requirePremium() call site through, even when not premium", () => {
    useEntitlementStore.setState({ premium: false });

    for (const feature of Object.keys(PREMIUM_FEATURES) as Array<
      keyof typeof PREMIUM_FEATURES
    >) {
      expect(requirePremium(feature)).toBe(true);
    }
  });

  it("never opens the checkout modal, even when a CTA asks it to", () => {
    render(<SupporterUpsellModal />);
    openSupporterUpsell("STEMS");
    openSupporterUpsell();

    expect(screen.queryByText("Become a Supporter")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Amount")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Already a Supporter? Restore purchase"),
    ).not.toBeInTheDocument();
  });
});

describe("entitlement store with payments disabled", () => {
  it("starts unlocked and settled, with no cache read", () => {
    expect(INITIAL_ENTITLEMENT.premium).toBe(true);
    expect(INITIAL_ENTITLEMENT.loading).toBe(false);
  });

  it("refresh() makes no entitlement request", async () => {
    await useEntitlementStore.getState().refresh();
    expect(accountEntitlements).not.toHaveBeenCalled();
  });
});

describe("Account card with payments disabled", () => {
  it("hides the plan row and the Supporter CTA", () => {
    render(<AccountCard />);

    expect(screen.queryByText("Plan")).not.toBeInTheDocument();
    expect(screen.queryByText("Free")).not.toBeInTheDocument();
    expect(screen.queryByText("Supporter")).not.toBeInTheDocument();
    expect(screen.queryByText("Become a Supporter")).not.toBeInTheDocument();
    // The non-payment account affordances are untouched.
    expect(screen.getByText("Attach email")).toBeInTheDocument();
  });

  it("drops the donation-receipts pitch from the anonymous blurb", () => {
    render(<AccountCard />);
    expect(screen.queryByText(/donation receipts/i)).not.toBeInTheDocument();
  });
});
