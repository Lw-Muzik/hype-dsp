import { describe, expect, it } from "vitest";
import { downloadPercent, noticeFor } from "./updateNotice";

describe("noticeFor", () => {
  it("shows a determinate downloading pill", () => {
    expect(
      noticeFor({ state: "downloading", version: "0.1.30", received: 45, total: 100 }, null),
    ).toEqual({ kind: "pill", phase: "downloading", version: "0.1.30", percent: 45 });
  });

  it("shows an indeterminate pill when there is no total", () => {
    expect(
      noticeFor({ state: "downloading", version: "0.1.30", received: 45, total: null }, null),
    ).toEqual({ kind: "pill", phase: "downloading", version: "0.1.30", percent: null });
  });

  it("shows the banner for a ready update, with string notes", () => {
    expect(noticeFor({ state: "ready", version: "0.1.30", notes: "Fixes" }, null)).toEqual({
      kind: "banner",
      version: "0.1.30",
      notes: "Fixes",
    });
  });

  it("keeps null notes as null", () => {
    expect(noticeFor({ state: "ready", version: "0.1.30", notes: null }, null)).toEqual({
      kind: "banner",
      version: "0.1.30",
      notes: null,
    });
  });

  it("collapses to a ready pill once that version is dismissed", () => {
    expect(noticeFor({ state: "ready", version: "0.1.30", notes: "n" }, "0.1.30")).toEqual({
      kind: "pill",
      phase: "ready",
      version: "0.1.30",
      notes: "n",
    });
  });

  it("brings the banner back for a newer version than the dismissed one", () => {
    expect(noticeFor({ state: "ready", version: "0.1.31", notes: null }, "0.1.30")?.kind).toBe(
      "banner",
    );
  });

  it("shows nothing for idle, checking or failed", () => {
    expect(noticeFor({ state: "idle" }, null)).toBeNull();
    expect(noticeFor({ state: "checking" }, null)).toBeNull();
    expect(noticeFor({ state: "failed", message: "offline" }, null)).toBeNull();
  });
});

describe("downloadPercent (re-checked at its new home)", () => {
  it("is null without a total", () => {
    expect(downloadPercent(5, null)).toBeNull();
  });
});
