/**
 * Retired: core playback, library, basic EQ, Stations, visualizers, and
 * YouTube Music are free forever now — there's no trial to count down (see
 * `SessionGate`, which replaced the old sign-in-wall `AuthGate`). Kept as a
 * no-op rather than deleted so `App.tsx` doesn't need touching if a future
 * "renewal due" style notice ever needs this exact slot back.
 */
export function TrialBanner() {
  return null;
}
