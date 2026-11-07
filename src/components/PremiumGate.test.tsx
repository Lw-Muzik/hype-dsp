// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PremiumGate, requirePremium } from "@/components/PremiumGate";
import { closePlusSheet, PlusSheet } from "@/components/PlusSheet";
import { PREMIUM_FEATURES, useEntitlementStore } from "@/stores/entitlement";
import { useAccountStore } from "@/stores/account";
import { PLUS_FEATURES } from "@/config/payments";
import type { AccountStatus } from "@/lib/types";

/**
 * Per-feature gating: the six Plus features are locked unless entitled;
 * everything else always passes. (The flags are real, not mocked.)
 */
const ALL = Object.keys(PREMIUM_FEATURES) as Array<keyof typeof PREMIUM_FEATURES>;
const PLUS = ["PER_APP_EQ", "PER_APP_MIXER", "CONVOLVER", "SURROUND_3D", "ROOM_REVERB", "VIPER_DDC"] as const;
const FREE = ALL.filter((f) => !(PLUS as readonly string[]).includes(f));

function setEntitlement(premium: boolean, supporter: boolean, isAdmin = false) {
  useEntitlementStore.setState({ premium, supporter, loading: false });
  useAccountStore.setState({ status: isAdmin ? ({ isAdmin: true } as AccountStatus) : null });
}

beforeEach(() => {
  localStorage.clear();
  setEntitlement(false, false);
  useEntitlementStore.setState({ trialEndsAt: null });
});
afterEach(() => {
  act(() => closePlusSheet());
  cleanup();
});

describe("PLUS_FEATURES", () => {
  it("is exactly the six Plus features; the always-free ones are not in it", () => {
    expect([...PLUS_FEATURES].sort()).toEqual([...PLUS].sort());
    for (const f of ["SYSTEM_EQ", "AUTOEQ", "STEMS", "CLOUD"] as const) {
      expect(PLUS_FEATURES.has(f)).toBe(false);
    }
  });
});

describe("requirePremium()", () => {
  it("locks every Plus feature when not entitled, and opens the Plus sheet", () => {
    render(<PlusSheet />);
    act(() => {
      for (const f of PLUS) expect(requirePremium(f)).toBe(false);
    });
    expect(screen.getByRole("dialog", { name: "HypeMuzik Plus" })).toBeInTheDocument();
  });

  it("always passes free features (SYSTEM_EQ, AUTOEQ, STEMS, CLOUD) and opens no sheet", () => {
    render(<PlusSheet />);
    for (const f of FREE) expect(requirePremium(f)).toBe(true);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it.each([
    ["premium (admin/trial/licence)", () => setEntitlement(true, false)],
    ["supporter (a Plus purchase)", () => setEntitlement(false, true)],
    ["local admin", () => setEntitlement(false, false, true)],
  ])("passes every feature for %s", (_name, arrange) => {
    arrange();
    for (const f of ALL) expect(requirePremium(f)).toBe(true);
  });
});

describe("<PremiumGate>", () => {
  it("shows the Plus upsell for a locked Plus feature", () => {
    render(
      <PremiumGate feature="SURROUND_3D">
        <div>inner</div>
      </PremiumGate>,
    );
    expect(screen.queryByText("inner")).not.toBeInTheDocument();
    expect(screen.getByText("3D Surround is part of HypeMuzik Plus")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Get Plus" })).toBeInTheDocument();
  });

  it("renders children for free features without an upsell", () => {
    for (const f of FREE) {
      const { unmount } = render(
        <PremiumGate feature={f}>
          <div>{`open:${f}`}</div>
        </PremiumGate>,
      );
      expect(screen.getByText(`open:${f}`)).toBeInTheDocument();
      expect(screen.queryByText(/part of HypeMuzik Plus/)).not.toBeInTheDocument();
      unmount();
    }
  });

  it("unlocks live when entitlement arrives", () => {
    render(
      <PremiumGate feature="PER_APP_MIXER" variant="page">
        <div>mixer</div>
      </PremiumGate>,
    );
    expect(screen.queryByText("mixer")).not.toBeInTheDocument();
    act(() => setEntitlement(false, true));
    expect(screen.getByText("mixer")).toBeInTheDocument();
  });
});

describe("<PremiumGate> after a trial ended", () => {
  it("says so the first time only, then falls back to the normal copy", () => {
    useEntitlementStore.setState({ trialEndsAt: new Date(Date.now() - 1000).toISOString() });
    const first = render(
      <PremiumGate feature="ROOM_REVERB">
        <div>x</div>
      </PremiumGate>,
    );
    expect(screen.getByText("Your 7-day Plus trial has ended")).toBeInTheDocument();
    first.unmount();
    render(
      <PremiumGate feature="ROOM_REVERB">
        <div>x</div>
      </PremiumGate>,
    );
    expect(screen.getByText("Room reverb is part of HypeMuzik Plus")).toBeInTheDocument();
  });

  it("uses the normal copy when there is no trial info (older backend)", () => {
    render(
      <PremiumGate feature="ROOM_REVERB">
        <div>x</div>
      </PremiumGate>,
    );
    expect(screen.getByText("Room reverb is part of HypeMuzik Plus")).toBeInTheDocument();
  });
});
