// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PremiumGate, requirePremium, SupporterUpsellModal } from "@/components/PremiumGate";
import { useEntitlementStore } from "@/stores/entitlement";
import { useToastStore } from "@/stores/toast";

beforeEach(() => {
  useEntitlementStore.setState({ premium: false, loading: false });
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  cleanup();
});

describe("<PremiumGate>", () => {
  it("renders children when the account is a Supporter", () => {
    useEntitlementStore.setState({ premium: true });

    render(
      <PremiumGate feature="STEMS">
        <div>Stems UI</div>
      </PremiumGate>,
    );

    expect(screen.getByText("Stems UI")).toBeInTheDocument();
    expect(screen.queryByText("Become a Supporter")).not.toBeInTheDocument();
  });

  it("renders the upsell instead of children when locked", () => {
    useEntitlementStore.setState({ premium: false });

    render(
      <PremiumGate feature="STEMS">
        <div>Stems UI</div>
      </PremiumGate>,
    );

    expect(screen.queryByText("Stems UI")).not.toBeInTheDocument();
    expect(screen.getByText(/Stem separation is a Supporter feature/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Become a Supporter/i })).toBeInTheDocument();
  });

  it("flips from upsell to children the moment premium becomes true (a live re-render, not just first paint)", () => {
    useEntitlementStore.setState({ premium: false });
    render(
      <PremiumGate feature="CLOUD">
        <div>Cloud UI</div>
      </PremiumGate>,
    );
    expect(screen.queryByText("Cloud UI")).not.toBeInTheDocument();

    // Simulates the entitlement refresh landing while the view is open.
    act(() => {
      useEntitlementStore.setState({ premium: true });
    });

    expect(screen.getByText("Cloud UI")).toBeInTheDocument();
    expect(screen.queryByText(/is a Supporter feature/)).not.toBeInTheDocument();
  });

  it("clicking the upsell CTA opens the (stubbed) checkout toast", () => {
    useEntitlementStore.setState({ premium: false });
    render(
      <PremiumGate feature="AUTOEQ">
        <div>AutoEQ UI</div>
      </PremiumGate>,
    );

    fireEvent.click(screen.getByRole("button", { name: /Become a Supporter/i }));

    expect(
      useToastStore.getState().toasts.some((t) => /coming soon/i.test(t.message)),
    ).toBe(true);
  });
});

describe("requirePremium", () => {
  it("returns true without surfacing anything when already a Supporter", () => {
    useEntitlementStore.setState({ premium: true });
    expect(requirePremium("SYSTEM_EQ")).toBe(true);
  });

  it("returns false and opens the modal naming the locked feature", () => {
    useEntitlementStore.setState({ premium: false });
    render(<SupporterUpsellModal />);

    let result = true;
    act(() => {
      result = requirePremium("SYSTEM_EQ");
    });
    expect(result).toBe(false);

    expect(screen.getByText(/System-wide EQ/)).toBeInTheDocument();
    expect(screen.getByText(/is a Supporter feature/)).toBeInTheDocument();

    // Clean up the module-level upsell store so this doesn't leak into a
    // later test — closing via the real "Close" affordance, not a test-only
    // reset hook.
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
  });
});
