import type { QueueItem } from "@/stores/engine";

/**
 * Stable `uid`s for queue entries, so a controller can `jump` to an entry it
 * saw in an earlier `state` even after the queue has changed around it.
 *
 * The engine store keeps plain items in an array: removing one shifts the
 * rest, a radio page appends, a tag lookup replaces one item with a patched
 * copy (`{ ...item, title }`). An entry keeps its uid when it is the same
 * object (wherever it moved), or a patched copy of the item that sat at the
 * same index under the same id. Anything else is new.
 */
export class QueueUidTracker {
  private byItem = new WeakMap<QueueItem, string>();
  private prevQueue: readonly QueueItem[] = [];
  private prevUids: string[] = [];
  private nextId = 1;

  uidsFor(queue: readonly QueueItem[]): string[] {
    if (queue === this.prevQueue) return this.prevUids;
    const used = new Set<string>();
    const uids = queue.map((item, i) => {
      let uid = this.byItem.get(item);
      if (uid === undefined || used.has(uid)) {
        const prev = this.prevQueue[i];
        const prevUid = this.prevUids[i];
        uid =
          prev !== undefined &&
          prevUid !== undefined &&
          !used.has(prevUid) &&
          prev.id === item.id &&
          prev.source === item.source
            ? prevUid
            : `q-${this.nextId++}`;
      }
      used.add(uid);
      this.byItem.set(item, uid);
      return uid;
    });
    this.prevQueue = queue;
    this.prevUids = uids;
    return uids;
  }
}
