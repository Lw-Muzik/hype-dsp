import { requirePremium } from "@/components/PremiumGate";
import { ipcErrorMessage } from "@/lib/ipc";
import type { SystemEqScope, SystemEqScopeMode } from "@/lib/types";
import { useEngineStore } from "@/stores/engine";
import { useSystemEqStore } from "@/stores/systemEq";

/**
 * Shared per-app scope logic for system-wide EQ (Settings card + the
 * Equalizer-mode sidebar panel). Each function resolves to an error message,
 * or `null` on success / when a premium gate refused.
 */

/**
 * Commit the scope to the backend FIRST (awaited), then — if system EQ is on —
 * rebuild the tap through the systemEq store so the new scope takes effect. A
 * failed rebuild stops, records the error and clears `enabled` (store behaviour).
 */
export async function applySystemEqScope(next: SystemEqScope): Promise<string | null> {
  try {
    await useEngineStore.getState().setSystemEqScope(next);
  } catch (e) {
    return ipcErrorMessage(e);
  }
  const eq = useSystemEqStore.getState();
  if (eq.enabled) await eq.enable();
  return null;
}

export function setScopeMode(mode: SystemEqScopeMode): Promise<string | null> {
  if (!requirePremium("PER_APP_EQ")) return Promise.resolve(null);
  const scope = useEngineStore.getState().state.systemEqScope;
  return applySystemEqScope({ mode, apps: scope.apps });
}

export function toggleScopeApp(id: string): Promise<string | null> {
  if (!requirePremium("PER_APP_EQ")) return Promise.resolve(null);
  const scope = useEngineStore.getState().state.systemEqScope;
  const apps = scope.apps.includes(id)
    ? scope.apps.filter((a) => a !== id)
    : [...scope.apps, id];
  return applySystemEqScope({ mode: scope.mode, apps });
}

/** "All apps" / "Only N apps" / "All except N apps". */
export function scopeSummary(scope: SystemEqScope): string {
  const n = scope.apps.length;
  const apps = `${n} app${n === 1 ? "" : "s"}`;
  if (scope.mode === "only") return `Only ${apps}`;
  if (scope.mode === "except") return `All except ${apps}`;
  return "All apps";
}
