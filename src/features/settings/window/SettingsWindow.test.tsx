// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

const win = vi.hoisted(() => ({
  close: vi.fn(async () => {}),
  sectionHandler: null as null | ((section: string) => void),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: win.close }),
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({ label: "settings" }),
}));
// The backend never answers: every card renders its loading state.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => new Promise(() => {})),
  convertFileSrc: (p: string) => p,
}));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(async () => {}),
  emitTo: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    onSettingsSection: vi.fn(async (handler: (section: string) => void) => {
      win.sectionHandler = handler;
      return () => {
        win.sectionHandler = null;
      };
    }),
  };
});

import { SettingsWindow } from "./SettingsWindow";
import { SETTINGS_SECTIONS } from "./sections";

const scrollIntoView = vi.fn();

beforeEach(() => {
  win.close.mockClear();
  scrollIntoView.mockClear();
  Element.prototype.scrollIntoView = scrollIntoView;
  window.location.hash = "";
});
afterEach(cleanup);

const mount = async () => {
  render(<SettingsWindow />);
  // Let the onSettingsSection subscription settle.
  await act(async () => {});
};

describe("SettingsWindow", () => {
  it("renders every section anchor and a nav entry for each", async () => {
    await mount();
    for (const { id, label } of SETTINGS_SECTIONS) {
      expect(document.getElementById(id), id).not.toBeNull();
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("scrolls to a section when settings://section arrives", async () => {
    await mount();
    expect(win.sectionHandler).not.toBeNull();
    act(() => win.sectionHandler?.("cloud"));
    expect(scrollIntoView).toHaveBeenCalledOnce();
    expect(scrollIntoView.mock.instances[0]).toBe(document.getElementById("cloud"));
  });

  it("ignores unknown sections", async () => {
    await mount();
    act(() => win.sectionHandler?.("nope"));
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("scrolls to the section in the URL hash on mount", async () => {
    window.location.hash = "#youtube-music";
    await mount();
    expect(scrollIntoView.mock.instances[0]).toBe(document.getElementById("youtube-music"));
  });

  it("closes on Escape", async () => {
    await mount();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(win.close).toHaveBeenCalledOnce();
  });

  it("closes on Cmd/Ctrl+W", async () => {
    await mount();
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    expect(win.close).toHaveBeenCalledOnce();
  });

  it("does not close on Escape while an input is focused", async () => {
    await mount();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(win.close).not.toHaveBeenCalled();
    input.remove();
  });

  it("does not close on Escape while a dialog is open", async () => {
    await mount();
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    document.body.appendChild(dialog);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(win.close).not.toHaveBeenCalled();
    dialog.remove();
  });
});
