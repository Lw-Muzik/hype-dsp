import { useEffect } from "react";
import { sessionLoad, sessionSave } from "@/lib/ipc";
import { useEngineStore } from "@/stores/engine";
import { parseSession, snapshotSession } from "./playbackSession";

/** Queue/position-in-queue changes are saved at most this often. */
export const SESSION_SAVE_THROTTLE_MS = 500;
/**
 * While playing, the position is saved this often. Saving on every progress
 * tick would be a disk write per frame of the progress bar; this loses at
 * most a few seconds on a hard quit.
 */
export const POSITION_SAVE_MS = 5_000;

/**
 * Bring the last playback session back on launch, and keep it saved. Mount
 * once, in the main window (it owns playback).
 *
 * Nothing is saved until the restore has been attempted: the store starts
 * empty, and saving that first would erase the session about to be restored.
 */
export function useSessionPersistence(): void {
  useEffect(() => {
    let disposed = false;
    let ready = false;
    // `undefined` = nothing written yet this run; `null` = written "no session".
    let lastJson: string | null | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const save = () => {
      if (disposed || !ready) return;
      let json: string | null;
      try {
        const snap = snapshotSession(useEngineStore.getState());
        json = snap ? JSON.stringify(snap) : null;
      } catch {
        return;
      }
      if (json === lastJson) return;
      lastJson = json;
      void sessionSave(json).catch(() => {});
    };

    // Doesn't reset a pending save: the queue array is replaced often by
    // background tag enrichment, and a resetting debounce could starve.
    const schedule = () => {
      if (disposed || timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        save();
      }, SESSION_SAVE_THROTTLE_MS);
    };

    void sessionLoad()
      .then((text) => {
        if (disposed) return;
        const session = parseSession(text);
        if (session) useEngineStore.getState().restoreSession(session);
      })
      .catch(() => {})
      .finally(() => {
        if (disposed) return;
        ready = true;
        schedule();
      });

    const unsubscribe = useEngineStore.subscribe((s, prev) => {
      if (
        s.queue !== prev.queue ||
        s.order !== prev.order ||
        s.orderPos !== prev.orderPos ||
        s.shuffle !== prev.shuffle ||
        s.repeat !== prev.repeat ||
        s.resumeAt !== prev.resumeAt ||
        // Pausing is a natural "stopped here": keep the exact spot.
        (s.paused && !prev.paused)
      ) {
        schedule();
      }
    });

    const ticker = setInterval(() => {
      const s = useEngineStore.getState();
      if (s.playing && !s.paused) save();
    }, POSITION_SAVE_MS);

    // The window hiding (macOS close hides it) or going away: save the spot now.
    const flush = () => save();
    const onVisibility = () => {
      if (document.visibilityState === "hidden") flush();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", flush);
    window.addEventListener("beforeunload", flush);

    return () => {
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
      clearInterval(ticker);
      unsubscribe();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", flush);
      window.removeEventListener("beforeunload", flush);
    };
  }, []);
}
