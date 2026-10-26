import { describe, expect, it, vi } from "vitest";
import { INTRO_TOTAL, drawIntro, toRgb } from "./introCanvas";

describe("introCanvas", () => {
  it("parses colours", () => {
    expect(toRgb("#f5b40f", "0,0,0")).toBe("245,180,15");
    expect(toRgb("nope", "1,2,3")).toBe("1,2,3");
  });
  it("draws every phase, with the lockup drawn only after the pop", () => {
    const grad = { addColorStop: vi.fn() };
    const ctx = new Proxy({} as Record<string, unknown>, {
      get: (t, k: string) =>
        k === "createLinearGradient" || k === "createRadialGradient" ? () => grad
        : k === "measureText" ? () => ({ width: 100 })
        : (t[k] ??= vi.fn()),
      set: (t, k: string, v) => ((t[k] = v), true),
    }) as unknown as CanvasRenderingContext2D;
    const logo = {} as CanvasImageSource;
    const th = { gold: "1,2,3", ground: "#000", text: "#fff", logo, logoAspect: 1, family: "x" };
    const draw = ctx.drawImage as unknown as ReturnType<typeof vi.fn>;
    for (const t of [0.5, 1.4, 1.9, 2.0]) drawIntro(ctx, 400, 800, t, th);
    expect(draw).not.toHaveBeenCalled();
    drawIntro(ctx, 400, 800, 2.5, th);
    drawIntro(ctx, 400, 800, INTRO_TOTAL, th);
    drawIntro(ctx, 400, 800, 0, th, true);
    expect(draw).toHaveBeenCalledTimes(2 + 1);
  });
});
