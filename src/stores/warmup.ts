/**
 * Deferred, latest-wins scheduling for prefetch spawns.
 *
 * A click on a track can fire several yt-dlp processes (audio resolve, video
 * warmup, next-track warmup) that all contend for the CPU and network at the
 * exact moment the listener is waiting for sound. Deferring the optional ones
 * a few seconds costs nothing — tracks run minutes — and "latest wins per
 * key" means skip-spam replaces pending work instead of stacking it.
 */

const pending = new Map<string, ReturnType<typeof setTimeout>>();

/** Run `fn` after `delayMs`, replacing any pending work under the same key. */
export function scheduleWarmup(key: string, delayMs: number, fn: () => void): void {
  const prior = pending.get(key);
  if (prior != null) clearTimeout(prior);
  pending.set(
    key,
    setTimeout(() => {
      pending.delete(key);
      fn();
    }, delayMs),
  );
}

/** Drop everything pending (playback stopped — nothing is worth warming). */
export function cancelWarmups(): void {
  for (const id of pending.values()) clearTimeout(id);
  pending.clear();
}
