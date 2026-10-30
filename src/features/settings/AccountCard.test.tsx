// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AccountCard } from "@/features/settings/AccountCard";
import { useAccountStore } from "@/stores/account";
import { useEntitlementStore } from "@/stores/entitlement";
import type { AccountStatus, LicenseInfo } from "@/lib/types";

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

/** Render the card with a given account status + entitlement flags. */
function mount(account: AccountStatus, supporter: boolean, premium = supporter) {
  useAccountStore.setState({ status: account });
  useEntitlementStore.setState({
    premium,
    supporter,
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
    supporter: false,
    loading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
  });
});

afterEach(() => cleanup());

describe("<AccountCard> plan states", () => {
  it("admin: shows an Admin badge, hides the purchase CTA, notes full access", () => {
    mount(status({ isAdmin: true, license: license({ state: "licensed" }) }), false, true);

    expect(screen.getByText("Admin")).toBeInTheDocument();
    expect(upsellButton()).not.toBeInTheDocument();
    expect(screen.getByText(/Full access to every feature/i)).toBeInTheDocument();
    expect(screen.queryByText("Free")).not.toBeInTheDocument();
    expect(screen.queryByText("Supporter")).not.toBeInTheDocument();
  });

  it("trial: the old trial wording is gone — premium-but-not-supporter reads as Free", () => {
    mount(status({ license: license({ state: "trial", daysLeft: 7 }) }), false, true);

    expect(screen.queryByText(/Trial/)).not.toBeInTheDocument();
    expect(screen.queryByText(/free trial/i)).not.toBeInTheDocument();
    expect(screen.getByText("Free")).toBeInTheDocument();
    expect(upsellButton()).toBeInTheDocument();
  });

  it("free / expired: shows Free and the purchase CTA", () => {
    mount(status({ license: license({ state: "expired", daysLeft: 0 }) }), false);

    expect(screen.getByText("Free")).toBeInTheDocument();
    expect(upsellButton()).toBeInTheDocument();
    expect(screen.getByText(/Every feature is free/i)).toBeInTheDocument();
  });

  it("supporter: shows Supporter and hides the purchase CTA", () => {
    mount(status({ license: license({ state: "licensed" }) }), true);

    expect(screen.getByText("Supporter")).toBeInTheDocument();
    expect(upsellButton()).not.toBeInTheDocument();
  });
});

describe("<AccountCard> anonymous install", () => {
  it("still shows the plan line and the Supporter button", () => {
    mount(status({ emailVerified: false, email: null, name: null }), false);

    expect(screen.getByText("Free")).toBeInTheDocument();
    expect(upsellButton()).toBeInTheDocument();
    expect(screen.getByText("Attach email")).toBeInTheDocument();
  });
});
