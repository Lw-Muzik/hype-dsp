import { describe, expect, it } from "vitest";
import { downloadPercent } from "./UpdateRow";

/**
 * The only real decision in the update UI: a chunked response carries no
 * `Content-Length`, so there is no total to divide by.
 *
 * Treating that as zero gives a progress bar pinned at 0% for the whole
 * download — which reads as a stall, at exactly the moment the user is waiting
 * on tens of megabytes. `null` is the signal to show an indeterminate bar
 * instead, and that difference is worth pinning down.
 */
describe("downloadPercent", () => {
  it("reports progress when the server sent a length", () => {
    expect(downloadPercent(0, 100)).toBe(0);
    expect(downloadPercent(50, 100)).toBe(50);
    expect(downloadPercent(100, 100)).toBe(100);
  });

  it("is null when there is no total, rather than zero", () => {
    expect(downloadPercent(1_000_000, null)).toBeNull();
  });

  /** A zero or negative total is a broken header, not a finished download.
   *  Dividing by it yields Infinity or NaN, both of which render as garbage. */
  it("treats a nonsense total as no total at all", () => {
    expect(downloadPercent(10, 0)).toBeNull();
    expect(downloadPercent(10, -1)).toBeNull();
  });

  /** Servers can send slightly more than they promised (trailing bytes, or a
   *  length that excluded encoding). The bar must not exceed full. */
  it("never exceeds 100 when more arrives than was promised", () => {
    expect(downloadPercent(120, 100)).toBe(100);
  });
});
