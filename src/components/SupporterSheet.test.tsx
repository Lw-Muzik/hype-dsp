// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openSupporterSheet, SupporterSheet } from "@/components/SupporterSheet";
import { useEntitlementStore } from "@/stores/entitlement";
import { useToastStore } from "@/stores/toast";

/**
 * Only the network-hitting IPC calls are mocked; `ipcErrorMessage` (a pure
 * helper the component also imports) is kept real so error-path assertions
 * exercise the actual formatting logic.
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
    supporter: false,
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

describe("Supporter sheet content", () => {
  it("explains the deal: free, badge only, one-time, 14-day refund, amounts from $3", () => {
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    expect(screen.getByRole("dialog", { name: "Become a Supporter" })).toBeInTheDocument();
    expect(screen.getByText(/every feature unlocked/i)).toBeInTheDocument();
    expect(screen.getByText(/Nothing is ever locked/i)).toBeInTheDocument();
    expect(screen.getByText(/One-time payment, not a subscription/i)).toBeInTheDocument();
    expect(screen.getByText(/14 days/i)).toBeInTheDocument();
    for (const v of ["$3", "$5", "$10", "$25"]) {
      expect(screen.getByRole("button", { name: v })).toBeInTheDocument();
    }
    // Default is $5.
    expect(screen.getByRole("button", { name: /Support \$5/ })).toBeInTheDocument();
    expect(screen.queryByText(/is a Supporter feature/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Supporter checkout (amount picker)", () => {
  it("starts checkout for a quick-pick amount, opens it in the browser, and shows the confirming state", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_123");
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    fireEvent.click(screen.getByRole("button", { name: "$25" }));
    fireEvent.click(screen.getByRole("button", { name: /Support \$25/ }));

    await waitFor(() => expect(accountSupportCheckout).toHaveBeenCalledWith(25));
    expect(openExternalUrl).toHaveBeenCalledWith("https://checkout.paddle.com/txn_123");
    // The modal stays OPEN and switches to the "confirming…" view — the user
    // isn't dropped back with no feedback while the webhook is in flight.
    await waitFor(() =>
      expect(screen.getByText(/Confirming your payment/i)).toBeInTheDocument(),
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  });

  it("shows the thank-you view once entitlement flips to supporter after checkout", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_789");
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    // Default amount is $5 — check out straight away.
    fireEvent.click(screen.getByRole("button", { name: /Support \$5/ }));
    await waitFor(() =>
      expect(screen.getByText(/Confirming your payment/i)).toBeInTheDocument(),
    );

    // The webhook lands → entitlement refresh flips premium true.
    act(() => useEntitlementStore.setState({ supporter: true }));

    expect(screen.getByText(/thank you/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
  });

  it("uses a typed custom amount instead of the quick picks", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_456");
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    fireEvent.change(screen.getByLabelText("Custom amount"), { target: { value: "42" } });
    fireEvent.click(screen.getByRole("button", { name: /Support \$42/ }));

    await waitFor(() => expect(accountSupportCheckout).toHaveBeenCalledWith(42));
  });

  it("disables checkout below the $3 minimum instead of sending a bad amount", () => {
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    fireEvent.change(screen.getByLabelText("Custom amount"), { target: { value: "2" } });

    expect(screen.getByRole("button", { name: "Support" })).toBeDisabled();
    expect(accountSupportCheckout).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("shows a failed checkout plainly in the sheet, re-enables the button and keeps it open", async () => {
    vi.mocked(accountSupportCheckout).mockRejectedValue({
      code: "account",
      message: "Checkout is not available yet",
    });
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    fireEvent.click(screen.getByRole("button", { name: "$5" }));
    fireEvent.click(screen.getByRole("button", { name: /Support \$5/ }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/Checkout is not available yet/),
    );
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    // No stuck spinner: the button is back and usable for a retry.
    expect(screen.getByRole("button", { name: /Support \$5/ })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Restore Supporter", () => {
  it("restores a Supporter entitlement from a receipt email and refreshes entitlement", async () => {
    vi.mocked(accountSupportRestore).mockResolvedValue(true);
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

    fireEvent.click(screen.getByRole("button", { name: /Already a Supporter/i }));
    expect(screen.getByText(/hypemuzik\.com\/support/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Receipt email"), {
      target: { value: "supporter@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() =>
      expect(accountSupportRestore).toHaveBeenCalledWith("supporter@example.com"),
    );
    // A successful restore refreshes entitlement; the thank-you is handled
    // centrally by the unlock watcher (no duplicate "restored" toast here).
    await waitFor(() => expect(useEntitlementStore.getState().refresh).toHaveBeenCalled());
    expect(useToastStore.getState().toasts.some((t) => /restored/i.test(t.message))).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("tells the user when no match is found, without treating it as an error", async () => {
    vi.mocked(accountSupportRestore).mockResolvedValue(false);
    render(<SupporterSheet />);
    act(() => openSupporterSheet());

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

describe("Supporter thank-you", () => {
  const isUnlockToast = (m: string) => /you're a supporter/i.test(m);

  it("toasts once when entitlement transitions to Supporter mid-session", () => {
    // First check has already settled at not-premium (loading:false in setup).
    render(<SupporterSheet />);
    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);

    act(() => useEntitlementStore.setState({ supporter: true }));

    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(true);
  });

  it("stays silent on the cache→server hydration flip for an existing supporter", () => {
    // Launch: still loading, cache not yet premium.
    useEntitlementStore.setState({ supporter: false, loading: true });
    render(<SupporterSheet />);

    // The first successful check settles as premium (an existing supporter) —
    // this is hydration, not a fresh unlock, so it must NOT toast.
    act(() => useEntitlementStore.setState({ supporter: true, loading: false }));

    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);
  });

  it("does not thank (or badge) an account that is merely premium, e.g. admin/trial", () => {
    render(<SupporterSheet />);
    act(() => useEntitlementStore.setState({ premium: true }));
    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);
  });
});
