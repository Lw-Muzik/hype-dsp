import type { SystemAudioStatus } from "../../lib/ipc";

/** Which affordance the System-wide audio card shows for a given status. */
export type SystemAudioAffordance =
  | "enable" // ready now → Enable / Restart / Stop
  | "install" // driver bundled but not installed → "Install audio driver"
  | "not-bundled" // no signed driver in this build → one-click VB-CABLE setup
  | "unavailable"; // OS backend absent (no tap / no PulseAudio) → plain notice

/**
 * Decide the card's affordance from {@link SystemAudioStatus}.
 *
 * The order matters: `available` wins outright (an already-working pipeline is
 * never asked to install anything), and "Install audio driver" is offered only
 * when there is actually a bundled package to install — a driverless build
 * showing that button could only dead-end in an error.
 */
export function systemAudioAffordance(
  status: SystemAudioStatus,
): SystemAudioAffordance {
  if (status.available) return "enable";
  if (status.needsDriver && status.driverBundled) return "install";
  if (!status.driverBundled) return "not-bundled";
  return "unavailable";
}
