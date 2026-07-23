import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cancelWarmups, scheduleWarmup } from "@/stores/warmup";

describe("scheduleWarmup", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    cancelWarmups();
    vi.useRealTimers();
  });

  it("runs the work only after the delay", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 3000, fn);
    vi.advanceTimersByTime(2999);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("latest wins per key — skip-spam never stacks spawns", () => {
    const a = vi.fn();
    const b = vi.fn();
    scheduleWarmup("video", 3000, a);
    vi.advanceTimersByTime(1500);
    scheduleWarmup("video", 3000, b);
    vi.advanceTimersByTime(3000);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("keys are independent — the video warmup does not cancel the batch", () => {
    const video = vi.fn();
    const batch = vi.fn();
    scheduleWarmup("video", 3000, video);
    scheduleWarmup("batch", 3000, batch);
    vi.advanceTimersByTime(3000);
    expect(video).toHaveBeenCalledTimes(1);
    expect(batch).toHaveBeenCalledTimes(1);
  });

  it("cancelWarmups drops everything pending", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 3000, fn);
    cancelWarmups();
    vi.advanceTimersByTime(10000);
    expect(fn).not.toHaveBeenCalled();
  });

  it("a fired warmup does not linger — rescheduling after it fires works", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 1000, fn);
    vi.advanceTimersByTime(1000);
    scheduleWarmup("video", 1000, fn);
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});
