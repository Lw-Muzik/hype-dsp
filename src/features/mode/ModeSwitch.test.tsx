// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("@/lib/ipc", () => ({ systemAudioStatus: vi.fn() }));
vi.mock("./switchMode", () => ({ switchMode: vi.fn(async () => "switched") }));

import { systemAudioStatus } from "@/lib/ipc";
import { useSystemEqStore } from "@/stores/systemEq";
import { ModeSwitch } from "./ModeSwitch";
import { switchMode } from "./switchMode";

beforeEach(() => {
  vi.mocked(systemAudioStatus).mockResolvedValue({ supported: true } as never);
  useSystemEqStore.setState({ enabled: false });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ModeSwitch", () => {
  it("renders both options with aria-checked following the mode", () => {
    const { rerender } = render(<ModeSwitch />);
    expect(screen.getByRole("radiogroup")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Player" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: "Equalizer" })).toHaveAttribute("aria-checked", "false");
    useSystemEqStore.setState({ enabled: true });
    rerender(<ModeSwitch />);
    expect(screen.getByRole("radio", { name: "Equalizer" })).toHaveAttribute("aria-checked", "true");
  });

  it("is disabled with a tooltip when unsupported", async () => {
    vi.mocked(systemAudioStatus).mockResolvedValue({ supported: false } as never);
    render(<ModeSwitch />);
    const eq = screen.getByRole("radio", { name: "Equalizer" });
    await waitFor(() => expect(eq).toBeDisabled());
    expect(eq).toHaveAttribute("title", "Equalizer mode isn't available on this computer");
  });

  it("clicking a segment calls switchMode; the current one is a no-op", async () => {
    render(<ModeSwitch />);
    fireEvent.click(screen.getByRole("radio", { name: "Player" }));
    expect(switchMode).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("radio", { name: "Equalizer" }));
    expect(switchMode).toHaveBeenCalledWith("equalizer");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Equalizer" })).toBeEnabled());
  });

  it("arrow keys move focus between segments", () => {
    render(<ModeSwitch />);
    const player = screen.getByRole("radio", { name: "Player" });
    player.focus();
    fireEvent.keyDown(player, { key: "ArrowRight" });
    expect(screen.getByRole("radio", { name: "Equalizer" })).toHaveFocus();
  });
});
