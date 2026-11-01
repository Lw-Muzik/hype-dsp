// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

const ev = vi.hoisted(() => ({
  openHandler: null as null | (() => void),
}));

// The backend never answers: every card renders its loading state.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => new Promise(() => {})),
  convertFileSrc: (p: string) => p,
}));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(async () => {}),
  emitTo: vi.fn(async () => {}),
  listen: vi.fn(async (name: string, cb: () => void) => {
    if (name === "settings://open") ev.openHandler = cb;
    return () => {
      if (name === "settings://open") ev.openHandler = null;
    };
  }),
}));

import { SettingsDialog } from "./SettingsDialog";
import { SETTINGS_SECTIONS } from "./sections";
import { closeSettings, openSettings, useSettingsDialogStore } from "./settingsDialogStore";

const scrollIntoView = vi.fn();

beforeEach(() => {
  scrollIntoView.mockClear();
  Element.prototype.scrollIntoView = scrollIntoView;
  closeSettings();
});
afterEach(cleanup);

const mount = async () => {
  render(<SettingsDialog />);
  // Let the settings://open subscription settle.
  await act(async () => {});
};
const open = (section?: Parameters<typeof openSettings>[0]) => act(() => void openSettings(section));
const isOpen = () => useSettingsDialogStore.getState().open;

describe("SettingsDialog", () => {
  it("renders nothing while closed", async () => {
    await mount();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("Settings")).toBeNull();
  });

  it("renders the heading, nav and every section when open", async () => {
    await mount();
    open();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    for (const { id, label } of SETTINGS_SECTIONS) {
      expect(document.getElementById(id), id).not.toBeNull();
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("closes from the close button", async () => {
    await mount();
    open();
    fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
    expect(isOpen()).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes from a backdrop click", async () => {
    await mount();
    open();
    const backdrop = screen.getByRole("dialog").previousElementSibling as HTMLElement;
    fireEvent.click(backdrop);
    expect(isOpen()).toBe(false);
  });

  it("closes on Escape", async () => {
    await mount();
    open();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(isOpen()).toBe(false);
  });

  it("does not close on Escape while another dialog is open", async () => {
    await mount();
    open();
    const other = document.createElement("div");
    other.setAttribute("role", "dialog");
    document.body.appendChild(other);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(isOpen()).toBe(true);
    other.remove();
  });

  it("does not close on Escape while an input is focused", async () => {
    await mount();
    open();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(isOpen()).toBe(true);
    input.remove();
  });

  it("closes on Cmd/Ctrl+W, and ignores it while closed", async () => {
    await mount();
    const closedEvent = new KeyboardEvent("keydown", { key: "w", metaKey: true, cancelable: true });
    window.dispatchEvent(closedEvent);
    expect(closedEvent.defaultPrevented).toBe(false);

    open();
    const e = new KeyboardEvent("keydown", { key: "w", metaKey: true, cancelable: true });
    window.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    expect(isOpen()).toBe(false);
  });

  it("opens at a section: jumps there, marks it current and moves focus in", async () => {
    await mount();
    open("cloud");
    expect(scrollIntoView).toHaveBeenCalledOnce();
    expect(scrollIntoView.mock.instances[0]).toBe(document.getElementById("cloud"));
    expect(screen.getByRole("button", { name: "Cloud" })).toHaveAttribute("aria-current", "true");
    expect(screen.getByRole("dialog")).toHaveFocus();
  });

  it("scrolls again when asked for a section while already open", async () => {
    await mount();
    open("cloud");
    scrollIntoView.mockClear();
    open("devices");
    expect(scrollIntoView).toHaveBeenCalledOnce();
    expect(scrollIntoView.mock.instances[0]).toBe(document.getElementById("devices"));
    open("devices");
    expect(scrollIntoView).toHaveBeenCalledTimes(2);
  });

  it("restores focus to the opener on close", async () => {
    await mount();
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    open();
    expect(opener).not.toHaveFocus();
    act(() => closeSettings());
    expect(opener).toHaveFocus();
    opener.remove();
  });

  it("opens when the native settings://open event arrives", async () => {
    await mount();
    expect(ev.openHandler).not.toBeNull();
    act(() => ev.openHandler?.());
    expect(isOpen()).toBe(true);
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  });
});
