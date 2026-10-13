import { describe, expect, it } from "vitest";
import { QueueUidTracker } from "./queueUids";
import { ytItem } from "./testKit";

describe("QueueUidTracker", () => {
  it("gives every entry a distinct uid and returns the cached array for the same queue", () => {
    const t = new QueueUidTracker();
    const q = [ytItem("a"), ytItem("b"), ytItem("c")];
    const ids = t.uidsFor(q);
    expect(new Set(ids).size).toBe(3);
    expect(t.uidsFor(q)).toBe(ids);
  });

  it("keeps an entry's uid when it moves (same object)", () => {
    const t = new QueueUidTracker();
    const [a, b, c] = [ytItem("a"), ytItem("b"), ytItem("c")] as const;
    const [ua, ub, uc] = t.uidsFor([a, b, c]) as [string, string, string];
    expect(t.uidsFor([c, a, b])).toEqual([uc, ua, ub]);
  });

  it("keeps the rest stable when an entry is removed", () => {
    const t = new QueueUidTracker();
    const [a, b, c] = [ytItem("a"), ytItem("b"), ytItem("c")] as const;
    const [ua, , uc] = t.uidsFor([a, b, c]) as [string, string, string];
    expect(t.uidsFor([a, c])).toEqual([ua, uc]);
  });

  it("gives appended entries fresh uids that are never reused", () => {
    const t = new QueueUidTracker();
    const [a, b] = [ytItem("a"), ytItem("b")] as const;
    const first = t.uidsFor([a, b]);
    const d = ytItem("d");
    const next = t.uidsFor([a, b, d]);
    expect(next.slice(0, 2)).toEqual(first);
    expect(first).not.toContain(next[2]);
    // dropping it and appending something else never recycles its uid
    t.uidsFor([a, b]);
    const again = t.uidsFor([a, b, ytItem("e")]);
    expect(again[2]).not.toBe(next[2]);
  });

  it("a patched copy (same id and source, same index) keeps its uid", () => {
    const t = new QueueUidTracker();
    const [a, b] = [ytItem("a"), ytItem("b")] as const;
    const [ua, ub] = t.uidsFor([a, b]) as [string, string];
    expect(t.uidsFor([{ ...a, title: "Tagged title" }, b])).toEqual([ua, ub]);
  });

  it("a different item at the same index does not inherit the uid", () => {
    const t = new QueueUidTracker();
    const [ua] = t.uidsFor([ytItem("a")]) as [string];
    expect(t.uidsFor([ytItem("z")])[0]).not.toBe(ua);
  });

  it("two copies of one patched item never share a uid", () => {
    const t = new QueueUidTracker();
    const a = ytItem("a");
    const [ua] = t.uidsFor([a]) as [string];
    const ids = t.uidsFor([a, { ...a }]);
    expect(ids[0]).toBe(ua);
    expect(new Set(ids).size).toBe(2);
  });

  it("the same object queued twice still yields distinct uids on every pass", () => {
    const t = new QueueUidTracker();
    const a = ytItem("a");
    t.uidsFor([a, a]);
    const again = t.uidsFor([a, a, a].slice(0, 2));
    expect(new Set(again).size).toBe(2);
    const third = t.uidsFor([a, a]);
    expect(new Set(third).size).toBe(2);
  });
});
