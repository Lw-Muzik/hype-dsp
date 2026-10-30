// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/** The store reads localStorage when its module loads, so load it fresh. */
async function freshStore() {
  vi.resetModules();
  return (await import("@/stores/ui")).useUiStore;
}

beforeEach(() => localStorage.clear());

describe("ui store remembers where the user was", () => {
  it("reopens on the last view, panel and sidebar state", async () => {
    let ui = await freshStore();
    ui.getState().setRoute("stations");
    ui.getState().toggleRight("lyrics");
    ui.getState().toggleSidebar();

    ui = await freshStore();
    expect(ui.getState().route).toBe("stations");
    expect(ui.getState().rightPanel).toBe("lyrics");
    expect(ui.getState().sidebarCollapsed).toBe(true);
  });

  it("falls back safely from an unknown or removed route", async () => {
    localStorage.setItem("hm.route", "settings"); // Settings is a window now
    expect((await freshStore()).getState().route).toBe("player");
    localStorage.setItem("hm.route", "movies"); // Movies was removed — land on Library
    expect((await freshStore()).getState().route).toBe("player");
    localStorage.setItem("hm.route", "<script>");
    expect((await freshStore()).getState().route).toBe("player");
  });

  it("doesn't reopen the Video tab (no video is loaded after a relaunch)", async () => {
    localStorage.setItem("hm.rightPanel", "video");
    expect((await freshStore()).getState().rightPanel).toBeNull();
  });

  it("closing the panel is remembered too", async () => {
    let ui = await freshStore();
    ui.getState().toggleRight("queue");
    ui.getState().closeRight();
    ui = await freshStore();
    expect(ui.getState().rightPanel).toBeNull();
  });
});
