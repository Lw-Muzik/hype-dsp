import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { SETTINGS_SECTIONS } from "@/features/settings/window/sections";

/** The Rust allow-list and the TS union must name the same sections. */
describe("settings section ids", () => {
  it("match src-tauri/src/commands/settings_window.rs", () => {
    const rust = readFileSync("src-tauri/src/commands/settings_window.rs", "utf8");
    const block = /pub const SECTIONS: &\[&str\] = &\[([^\]]*)\]/.exec(rust)?.[1] ?? "";
    const rustIds = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(rustIds).toEqual(SETTINGS_SECTIONS.map((s) => s.id));
  });
});
