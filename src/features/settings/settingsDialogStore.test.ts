import { beforeEach, describe, expect, it } from "vitest";
import { closeSettings, openSettings, useSettingsDialogStore } from "./settingsDialogStore";

const state = () => useSettingsDialogStore.getState();

beforeEach(() => closeSettings());

describe("settingsDialog store", () => {
  it("opens with a section", async () => {
    await openSettings("cloud");
    expect(state().open).toBe(true);
    expect(state().section).toBe("cloud");
  });

  it("opens at the top without a section", async () => {
    await openSettings();
    expect(state().open).toBe(true);
    expect(state().section).toBeNull();
  });

  it("re-requesting the same section is observable", async () => {
    await openSettings("cloud");
    const first = state().requestId;
    await openSettings("cloud");
    expect(state().requestId).toBe(first + 1);
    expect(state().section).toBe("cloud");
  });

  it("closes and forgets the section", async () => {
    await openSettings("devices");
    closeSettings();
    expect(state().open).toBe(false);
    expect(state().section).toBeNull();
  });
});
