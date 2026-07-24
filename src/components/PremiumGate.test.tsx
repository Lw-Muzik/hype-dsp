// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import {
  openSupporterUpsell,
  PremiumGate,
  requirePremium,
  SupporterUpsellModal,
} from "@/components/PremiumGate";
import { useEntitlementStore } from "@/stores/entitlement";
import { useToastStore } from "@/stores/toast";

/**
 * Only the three network-hitting IPC calls are mocked; `ipcErrorMessage` (a
 * pure helper the component also imports from this module) is kept real via
 * `importOriginal` so error-path assertions exercise the actual formatting
 * logic instead of a hand-rolled stand-in.
 */
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    accountSupportCheckout: vi.fn(),
    accountSupportRestore: vi.fn(),
    openExternalUrl: vi.fn(),
  };
});

import { accountSupportCheckout, accountSupportRestore, openExternalUrl } from "@/lib/ipc";

beforeEach(() => {
  useEntitlementStore.setState({
    premium: false,
    loading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
  });
  useToastStore.setState({ toasts: [] });
  vi.mocked(accountSupportCheckout).mockReset();
  vi.mocked(accountSupportRestore).mockReset();
  vi.mocked(openExternalUrl).mockReset().mockResolvedValue(undefined);
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

  it("clicking the upsell CTA opens the shared Supporter modal, naming the locked feature", () => {
    useEntitlementStore.setState({ premium: false });
    render(
      <>
        <PremiumGate feature="AUTOEQ">
          <div>AutoEQ UI</div>
        </PremiumGate>
        <SupporterUpsellModal />
      </>,
    );

    fireEvent.click(screen.getByRole("button", { name: /Become a Supporter/i }));

    expect(screen.getByRole("dialog", { name: "Become a Supporter" })).toBeInTheDocument();
    expect(screen.getByText(/AutoEQ is a Supporter feature/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
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

describe("Supporter checkout (amount picker)", () => {
  it("starts checkout for a quick-pick amount and opens it in the system browser", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_123");
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell("STEMS"));

    fireEvent.click(screen.getByRole("button", { name: "$25" }));
    fireEvent.click(screen.getByRole("button", { name: /Support \$25/ }));

    await waitFor(() => expect(accountSupportCheckout).toHaveBeenCalledWith(25));
    expect(openExternalUrl).toHaveBeenCalledWith("https://checkout.paddle.com/txn_123");
    await waitFor(() =>
      expect(
        useToastStore
          .getState()
          .toasts.some((t) => /complete your supporter checkout/i.test(t.message)),
      ).toBe(true),
    );
    expect(useEntitlementStore.getState().refresh).toHaveBeenCalled();
    // A successful checkout closes the modal.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("uses a typed custom amount instead of the quick picks", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_456");
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell());

    fireEvent.change(screen.getByLabelText("Custom amount"), { target: { value: "42" } });
    fireEvent.click(screen.getByRole("button", { name: /Support \$42/ }));

    await waitFor(() => expect(accountSupportCheckout).toHaveBeenCalledWith(42));
  });

  it("disables checkout below the $1 minimum instead of sending a bad amount", () => {
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell());

    fireEvent.change(screen.getByLabelText("Custom amount"), { target: { value: "0" } });

    expect(screen.getByRole("button", { name: "Support" })).toBeDisabled();
    expect(accountSupportCheckout).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("surfaces a failed checkout as a toast and keeps the modal open", async () => {
    vi.mocked(accountSupportCheckout).mockRejectedValue({
      code: "account",
      message: "couldn't reach the server",
    });
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell("CLOUD"));

    fireEvent.click(screen.getByRole("button", { name: "$5" }));
    fireEvent.click(screen.getByRole("button", { name: /Support \$5/ }));

    await waitFor(() =>
      expect(
        useToastStore
          .getState()
          .toasts.some((t) => t.kind === "error" && /couldn't reach the server/i.test(t.message)),
      ).toBe(true),
    );
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Restore Supporter", () => {
  it("restores a Supporter entitlement from a receipt email and refreshes entitlement", async () => {
    vi.mocked(accountSupportRestore).mockResolvedValue(true);
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell());

    fireEvent.click(screen.getByRole("button", { name: /Already a Supporter/i }));
    fireEvent.change(screen.getByLabelText("Receipt email"), {
      target: { value: "supporter@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() =>
      expect(accountSupportRestore).toHaveBeenCalledWith("supporter@example.com"),
    );
    await waitFor(() =>
      expect(useToastStore.getState().toasts.some((t) => /restored/i.test(t.message))).toBe(true),
    );
    expect(useEntitlementStore.getState().refresh).toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("tells the user when no match is found, without treating it as an error", async () => {
    vi.mocked(accountSupportRestore).mockResolvedValue(false);
    render(<SupporterUpsellModal />);
    act(() => openSupporterUpsell());

    fireEvent.click(screen.getByRole("button", { name: /Already a Supporter/i }));
    fireEvent.change(screen.getByLabelText("Receipt email"), {
      target: { value: "nobody@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() =>
      expect(
        useToastStore.getState().toasts.some((t) => /no supporter checkout found/i.test(t.message)),
      ).toBe(true),
    );
    expect(useToastStore.getState().toasts.every((t) => t.kind !== "error")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});
