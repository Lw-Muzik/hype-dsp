import { requirePremium } from "@/components/PremiumGate";
import { systemAudioAffordance } from "@/features/settings/systemAudioCard";
import { openSettings, systemAudioStatus, type SystemAudioStatus } from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { useSystemEqStore } from "@/stores/systemEq";
import { useUiStore, type Route } from "@/stores/ui";
import type { AppMode } from "./appMode";

export type SwitchOutcome =
  | "switched"
  | "blocked"
  | "unsupported"
  | "needs-setup"
  | "failed"
  | "busy";

export interface SwitchDeps {
  requirePremium: (feature: "SYSTEM_EQ") => boolean;
  systemAudioStatus: () => Promise<SystemAudioStatus>;
  openSettings: (section: "system-audio") => Promise<void>;
  enable: () => Promise<void>;
  disable: () => Promise<void>;
  /** Current store snapshot after enable(). */
  eqState: () => { enabled: boolean; error: string | null };
  setRoute: (route: Route) => void;
  toast: { error: (m: string) => void; info: (m: string) => void };
}

const defaultDeps: SwitchDeps = {
  requirePremium,
  systemAudioStatus,
  openSettings: (s) => openSettings(s),
  enable: () => useSystemEqStore.getState().enable(),
  disable: () => useSystemEqStore.getState().disable(),
  eqState: () => useSystemEqStore.getState(),
  setRoute: (r) => useUiStore.getState().setRoute(r),
  toast,
};

let running = false;

/** Switch between Player and Equalizer mode (= system-wide EQ off/on). */
export async function switchMode(
  target: AppMode,
  deps: SwitchDeps = defaultDeps,
): Promise<SwitchOutcome> {
  if (running) return "busy";
  running = true;
  try {
    if (target === "player") {
      await deps.disable();
      return "switched";
    }
    if (!deps.requirePremium("SYSTEM_EQ")) return "blocked";
    let status: SystemAudioStatus | null = null;
    try {
      status = await deps.systemAudioStatus();
    } catch {
      status = null; // unknown — try enabling anyway
    }
    if (status) {
      if (!status.supported) {
        deps.toast.error("Equalizer mode isn't available on this computer.");
        return "unsupported";
      }
      if (systemAudioAffordance(status) !== "enable") {
        void deps.openSettings("system-audio").catch(() => {});
        deps.toast.info("Equalizer mode needs a one-time setup — finish it in Settings.");
        return "needs-setup";
      }
    }
    await deps.enable();
    const { enabled, error } = deps.eqState();
    if (!enabled) {
      deps.toast.error(`Couldn't switch to Equalizer mode: ${error ?? "unknown error"}`);
      return "failed";
    }
    deps.setRoute("equalizer");
    return "switched";
  } finally {
    running = false;
  }
}
