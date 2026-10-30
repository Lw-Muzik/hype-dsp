// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PremiumGate, requirePremium } from "@/components/PremiumGate";
import { SupporterSheet } from "@/components/SupporterSheet";
import { PREMIUM_FEATURES, useEntitlementStore } from "@/stores/entitlement";
import { useAccountStore } from "@/stores/account";
import { FEATURE_GATING_ENABLED } from "@/config/payments";

/**
 * Every feature is free. These assert what ships: gating is off, nothing is
 * ever locked, and no feature ever triggers an upsell — whatever the
 * entitlement says. (This file deliberately does not mock the flag.)
 */
beforeEach(() => {
  useAccountStore.setState({ status: null });
  useEntitlementStore.setState({ premium: false, supporter: false, loading: false });
});
afterEach(() => cleanup());

const FEATURES = Object.keys(PREMIUM_FEATURES) as Array<keyof typeof PREMIUM_FEATURES>;

describe("feature gating is permanently off", () => {
  it("flag is false", () => {
    expect(FEATURE_GATING_ENABLED).toBe(false);
  });

  it("<PremiumGate> renders children for every feature, for a free user", () => {
    for (const feature of FEATURES) {
      const { unmount } = render(
        <PremiumGate feature={feature}>
          <div>{`unlocked:${feature}`}</div>
        </PremiumGate>,
      );
      expect(screen.getByText(`unlocked:${feature}`)).toBeInTheDocument();
      expect(screen.queryByText(/is a Supporter feature/)).not.toBeInTheDocument();
      expect(screen.queryByText("Become a Supporter")).not.toBeInTheDocument();
      unmount();
    }
  });

  it("requirePremium() passes for every feature and opens no sheet", () => {
    render(<SupporterSheet />);
    for (const feature of FEATURES) expect(requirePremium(feature)).toBe(true);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
