import { useEffect } from "react";
import { isPlusEntitled, usePlusEntitled } from "@/components/PremiumGate";
import { applySystemEqScope } from "@/features/mode/systemEqScope";
import { useEngineStore } from "@/stores/engine";
import type { SystemEqScope } from "@/lib/types";

/**
 * Persisted Plus effects for an account that is not entitled.
 *
 * The audio engine persists its own state, so an effect that was switched on
 * back when everything was free (or by an entitled account that has since
 * lapsed) would keep running. For each Plus effect that is saved as active we:
 *
 *  - 3D Surround / Room reverb / Convolver: switch it OFF in the engine. Only
 *    the `enabled` flag changes; intensity, room settings and the loaded IR
 *    stay saved.
 *  - Per-app EQ scope (`mode` other than "all"): fall back to "all apps", the
 *    free behaviour, and remember the previous scope.
 *
 * What we turned off is remembered in `localStorage` (`hm.plusSuppressed`). As
 * soon as the account is entitled again — a purchase, a restore, a late
 * entitlement fetch after an offline start — exactly those effects are turned
 * back on, so nothing the user configured is lost.
 *
 * Entitlement is the offline-cached value, so a paying user who launches
 * offline is entitled from the first render and is never suppressed. Before
 * the very first answer (no cache, nothing fetched) the user is treated as not
 * entitled: effects are held off, not discarded, and come back with the answer.
 *
 * Not covered, by design:
 *  - ViPER DDC curves load as ordinary EQ bands (the engine keeps no marker
 *    that a curve came from a DDC file), so a previously loaded curve is just
 *    an EQ curve; only *applying* a DDC is gated (`requirePremium`).
 *  - Per-app mixer volume/mute is OS-level state set from the mixer page;
 *    the page is locked, there is nothing of ours to bypass.
 */

const LS_KEY = "hm.plusSuppressed";

export interface Suppressed {
  surround3d?: true;
  room?: true;
  convolver?: true;
  /** The per-app scope that was replaced by "all apps". */
  scope?: SystemEqScope;
}

export function readSuppressed(): Suppressed {
  try {
    const raw = localStorage.getItem(LS_KEY);
    return raw ? (JSON.parse(raw) as Suppressed) : {};
  } catch {
    return {};
  }
}

function writeSuppressed(s: Suppressed): void {
  try {
    if (Object.keys(s).length === 0) localStorage.removeItem(LS_KEY);
    else localStorage.setItem(LS_KEY, JSON.stringify(s));
  } catch {
    // No storage: the effects stay off until the user re-enables them.
  }
}

/** Turn off whichever Plus effects are saved as active, remembering them. */
function suppress(): void {
  const engine = useEngineStore.getState();
  const st = engine.state;
  const rec = readSuppressed();
  let changed = false;

  if (st.surround3d.enabled) {
    rec.surround3d = true;
    changed = true;
  }
  if (st.room.enabled) {
    rec.room = true;
    changed = true;
  }
  if (st.convolver.enabled) {
    rec.convolver = true;
    changed = true;
  }
  const scopeActive = st.systemEqScope.mode !== "all";
  if (scopeActive) {
    rec.scope = st.systemEqScope;
    changed = true;
  }
  if (!changed) return;

  // Record first so a crash between the two steps can only lose the bypass,
  // never the user's saved choice.
  writeSuppressed(rec);
  if (st.surround3d.enabled) engine.setSurround3d({ ...st.surround3d, enabled: false });
  if (st.room.enabled) engine.setRoom({ ...st.room, enabled: false });
  if (st.convolver.enabled) engine.setConvolver({ ...st.convolver, enabled: false });
  if (scopeActive) void applySystemEqScope({ mode: "all", apps: [] });
}

/** Put back what {@link suppress} turned off. */
function restore(): void {
  const rec = readSuppressed();
  if (Object.keys(rec).length === 0) return;
  writeSuppressed({});
  const engine = useEngineStore.getState();
  const st = engine.state;
  if (rec.surround3d) engine.setSurround3d({ ...st.surround3d, enabled: true });
  if (rec.room) engine.setRoom({ ...st.room, enabled: true });
  if (rec.convolver) engine.setConvolver({ ...st.convolver, enabled: true });
  // Only bring the old scope back if the user has not picked another one since.
  if (rec.scope && st.systemEqScope.mode === "all" && st.systemEqScope.apps.length === 0) {
    void applySystemEqScope(rec.scope);
  }
}

/** Reconcile the engine with the current entitlement. Idempotent. */
export function enforcePlus(): void {
  if (isPlusEntitled()) restore();
  else suppress();
}

/** Mount once in the main window: re-runs whenever entitlement or any Plus
 *  effect's saved state changes (including when the engine state hydrates). */
export function usePlusEnforcement(): void {
  const entitled = usePlusEntitled();
  const surround = useEngineStore((s) => s.state.surround3d.enabled);
  const room = useEngineStore((s) => s.state.room.enabled);
  const convolver = useEngineStore((s) => s.state.convolver.enabled);
  const scopeMode = useEngineStore((s) => s.state.systemEqScope.mode);
  useEffect(() => {
    enforcePlus();
  }, [entitled, surround, room, convolver, scopeMode]);
}
