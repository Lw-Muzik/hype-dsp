import { create } from "zustand";
import {
  ipcErrorMessage,
  playerPlaySystemAudio,
  stopSystemAudio,
  systemAudioAvailable,
} from "@/lib/ipc";
import { toast } from "@/stores/toast";

/**
 * System-wide EQ on/off state, persisted across launches.
 *
 * System-wide EQ is a session mode rather than a DSP parameter (on macOS it
 * stands up a Core Audio process tap; on Linux/Windows a virtual sink), so it
 * lives outside the engine's saved state. We persist only the user's *intent*
 * here and re-engage the engine to match on the next launch — see {@link resume}.
 */

const LS_KEY = "hm.systemEqEnabled";

const loadEnabled = (): boolean => {
  try {
    return localStorage.getItem(LS_KEY) === "1";
  } catch {
    return false;
  }
};

const saveEnabled = (on: boolean): void => {
  try {
    localStorage.setItem(LS_KEY, on ? "1" : "0");
  } catch {
    // Private mode / no storage — the preference just won't persist.
  }
};

interface SystemEqState {
  /** Persisted user intent: route all system audio through the chain. Reflects
   *  the toggle; the engine is (re)started to match. */
  enabled: boolean;
  /** Last enable/resume failure, for the settings UI to surface. */
  error: string | null;
  /** Guards {@link resume} so the launch re-engage runs at most once (React
   *  StrictMode mounts effects twice in dev). */
  resumeAttempted: boolean;
  /** Engage system-wide EQ now and remember the choice. */
  enable: () => Promise<void>;
  /** Stop system-wide EQ now and remember the choice. */
  disable: () => Promise<void>;
  /** On launch, re-engage iff the user had left it on. Idempotent; a no-op when
   *  the user hadn't enabled it. */
  resume: () => Promise<void>;
  clearError: () => void;
}

export const useSystemEqStore = create<SystemEqState>((set, get) => ({
  enabled: loadEnabled(),
  error: null,
  resumeAttempted: false,

  enable: async () => {
    try {
      await playerPlaySystemAudio();
      saveEnabled(true);
      set({ enabled: true, error: null });
    } catch (e) {
      saveEnabled(false);
      set({ enabled: false, error: ipcErrorMessage(e) });
    }
  },

  disable: async () => {
    try {
      await stopSystemAudio();
    } catch {
      // Best-effort: honor the user's intent to turn it off even if the stop
      // IPC errors (the engine tears the routing down on its own anyway).
    }
    saveEnabled(false);
    set({ enabled: false, error: null });
  },

  resume: async () => {
    if (get().resumeAttempted) return;
    set({ resumeAttempted: true });
    if (!loadEnabled()) return; // user hadn't left it on — nothing to resume
    try {
      if (!(await systemAudioAvailable())) {
        // Feature no longer available on this machine — forget the choice.
        saveEnabled(false);
        set({ enabled: false });
        return;
      }
      await playerPlaySystemAudio();
      set({ enabled: true, error: null });
      toast.info("Resumed system-wide equalization");
    } catch (e) {
      // Couldn't resume (e.g. the audio-capture permission was revoked): clear
      // the intent so we don't silently re-try every launch, and say why.
      const message = ipcErrorMessage(e);
      saveEnabled(false);
      set({ enabled: false, error: message });
      toast.error(`Couldn't resume system-wide EQ: ${message}`);
    }
  },

  clearError: () => set({ error: null }),
}));
