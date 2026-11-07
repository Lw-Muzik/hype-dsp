// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openPlusSheet, PlusSheet } from "@/components/PlusSheet";
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
    trialEndsAt: null,
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

describe("Plus sheet content", () => {
  it("states the fixed deal: title, price line, the six features, one button", () => {
    render(<PlusSheet />);
    act(() => openPlusSheet());

    expect(screen.getByRole("dialog", { name: "HypeMuzik Plus" })).toBeInTheDocument();
    expect(screen.getByText("US$5 · one-time · yours for life")).toBeInTheDocument();
    const list = screen.getByRole("list", { name: /included with plus/i });
    for (const label of [
      "Per-app EQ",
      "Per-app mixer",
      "Convolver",
      "3D Surround",
      "Room reverb",
      "ViPER DDC curves",
    ]) {
      expect(within(list).getByText(label)).toBeInTheDocument();
    }
    expect(within(list).getAllByRole("listitem")).toHaveLength(6);
    expect(screen.getByRole("button", { name: "Get Plus — $5" })).toBeInTheDocument();
    // No amount picker of any kind.
    expect(screen.queryByLabelText(/amount/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "$10" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("shows a 'You have Plus' state instead of the buy button when already owned", () => {
    useEntitlementStore.setState({ supporter: true });
    render(<PlusSheet />);
    act(() => openPlusSheet());

    expect(screen.getByText("You have Plus")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Get Plus/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
  });

  it("uses no banned purchase wording", () => {
    render(<PlusSheet />);
    act(() => openPlusSheet());
    const text = screen.getByRole("dialog").textContent ?? "";
    expect(text).not.toMatch(/donat|\btip\b|support the developer|support us|pay what you want|any amount|contribute/i);
    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Plus trial state", () => {
  const inDays = (d: number) => new Date(Date.now() + d * 86_400_000 + 3_600_000).toISOString();

  it("shows 'Plus trial · N days left' and keeps the buy button", () => {
    useEntitlementStore.setState({ premium: true, supporter: false, trialEndsAt: inDays(3) });
    render(<PlusSheet />);
    act(() => openPlusSheet());
    expect(screen.getByTestId("plus-trial")).toHaveTextContent("Plus trial · 4 days left");
    expect(screen.getByRole("button", { name: "Get Plus — $5" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("says the trial has ended when not entitled", () => {
    useEntitlementStore.setState({
      premium: false,
      supporter: false,
      trialEndsAt: new Date(Date.now() - 1000).toISOString(),
    });
    render(<PlusSheet />);
    act(() => openPlusSheet());
    expect(screen.getByTestId("plus-trial")).toHaveTextContent("Your 7-day Plus trial has ended");
    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });

  it("shows no trial line without a trialEndsAt", () => {
    useEntitlementStore.setState({ premium: true, supporter: false, trialEndsAt: null });
    render(<PlusSheet />);
    act(() => openPlusSheet());
    expect(screen.queryByTestId("plus-trial")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Plus checkout", () => {
  it("starts checkout WITHOUT an amount, opens it in the browser, and shows the confirming state", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_123");
    render(<PlusSheet />);
    act(() => openPlusSheet());

    fireEvent.click(screen.getByRole("button", { name: "Get Plus — $5" }));

    await waitFor(() => expect(accountSupportCheckout).toHaveBeenCalledTimes(1));
    expect(vi.mocked(accountSupportCheckout).mock.calls[0]).toEqual([]);
    expect(openExternalUrl).toHaveBeenCalledWith("https://checkout.paddle.com/txn_123");
    // The modal stays OPEN and switches to the "confirming..." view.
    await waitFor(() =>
      expect(screen.getByText(/Confirming your payment/i)).toBeInTheDocument(),
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  });

  it("shows the thank-you view once entitlement flips to Plus after checkout", async () => {
    vi.mocked(accountSupportCheckout).mockResolvedValue("https://checkout.paddle.com/txn_789");
    render(<PlusSheet />);
    act(() => openPlusSheet());

    fireEvent.click(screen.getByRole("button", { name: "Get Plus — $5" }));
    await waitFor(() =>
      expect(screen.getByText(/Confirming your payment/i)).toBeInTheDocument(),
    );

    // The webhook lands -> entitlement refresh flips supporter true.
    act(() => useEntitlementStore.setState({ supporter: true }));

    expect(screen.getByText(/You have Plus — thank you/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
  });

  it("shows a failed checkout plainly in the sheet, re-enables the button and keeps it open", async () => {
    vi.mocked(accountSupportCheckout).mockRejectedValue({
      code: "account",
      message: "Checkout is not available yet",
    });
    render(<PlusSheet />);
    act(() => openPlusSheet());

    fireEvent.click(screen.getByRole("button", { name: "Get Plus — $5" }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/Checkout is not available yet/),
    );
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Get Plus — $5" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Restore purchase", () => {
  it("restores a Plus purchase from the checkout email and refreshes entitlement", async () => {
    vi.mocked(accountSupportRestore).mockResolvedValue(true);
    render(<PlusSheet />);
    act(() => openPlusSheet());

    fireEvent.click(screen.getByRole("button", { name: /Already bought Plus/i }));
    expect(screen.getByText(/email you used at checkout/i)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Checkout email"), {
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
    render(<PlusSheet />);
    act(() => openPlusSheet());

    fireEvent.click(screen.getByRole("button", { name: /Already bought Plus/i }));
    fireEvent.change(screen.getByLabelText("Checkout email"), {
      target: { value: "nobody@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() =>
      expect(
        useToastStore.getState().toasts.some((t) => /no hypemuzik plus purchase found for that email/i.test(t.message)),
      ).toBe(true),
    );
    expect(useToastStore.getState().toasts.every((t) => t.kind !== "error")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Maybe later" }));
  });
});

describe("Plus thank-you", () => {
  const isUnlockToast = (m: string) => /hypemuzik plus is unlocked/i.test(m);

  it("toasts once when entitlement transitions to Plus mid-session", () => {
    // First check has already settled at not-premium (loading:false in setup).
    render(<PlusSheet />);
    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);

    act(() => useEntitlementStore.setState({ supporter: true }));

    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(true);
  });

  it("stays silent on the cache→server hydration flip for an existing supporter", () => {
    // Launch: still loading, cache not yet premium.
    useEntitlementStore.setState({ supporter: false, loading: true });
    render(<PlusSheet />);

    // The first successful check settles as premium (an existing supporter) —
    // this is hydration, not a fresh unlock, so it must NOT toast.
    act(() => useEntitlementStore.setState({ supporter: true, loading: false }));

    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);
  });

  it("does not thank (or badge) an account that is merely premium, e.g. admin/trial", () => {
    render(<PlusSheet />);
    act(() => useEntitlementStore.setState({ premium: true }));
    expect(useToastStore.getState().toasts.some((t) => isUnlockToast(t.message))).toBe(false);
  });
});
