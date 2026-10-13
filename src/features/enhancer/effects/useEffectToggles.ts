import {
  AudioWaveform,
  Bookmark,
  Building2,
  Flame,
  Gauge,
  Headphones,
  Orbit,
  ShieldCheck,
  SlidersVertical,
  Speaker,
  Terminal,
  Waves,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { requirePremium } from "@/components/PremiumGate";
import { useEngineStore } from "@/stores/engine";
import type { EngineState } from "@/lib/types";

export type EffectId =
  | "equalizer"
  | "presets"
  | "bass"
  | "surround"
  | "surround3d"
  | "room"
  | "convolver"
  | "compander"
  | "saturation"
  | "script"
  | "headphone"
  | "limiter";

export interface EffectDescriptor {
  id: EffectId;
  /** Short label for the rack tile. */
  label: string;
  /** Accessible name of the on/off switch (rack). */
  switchLabel: string;
  icon: LucideIcon;
  /** DOM id of the card (Equalizer lives on its own view, so it has none). */
  anchor: string;
}

const d = (
  id: EffectId,
  label: string,
  switchLabel: string,
  icon: LucideIcon,
): EffectDescriptor => ({ id, label, switchLabel, icon, anchor: `fx-${id}` });

/** Rack order. */
export const EFFECTS: readonly EffectDescriptor[] = [
  d("equalizer", "Equalizer", "Equalizer on", SlidersVertical),
  d("presets", "Presets", "Presets", Bookmark),
  d("bass", "Bass", "Bass boost on", Speaker),
  d("surround", "Surround", "Surround on", Waves),
  d("surround3d", "3D Surround", "3D Surround on", Orbit),
  d("room", "Room", "Room reverb on", Building2),
  d("convolver", "Convolver", "Convolver on", AudioWaveform),
  d("compander", "Compander", "Multiband compander on", Gauge),
  d("saturation", "Saturation", "Tube saturation on", Flame),
  d("script", "Script", "LiveProg script on", Terminal),
  d("headphone", "Headphone", "Headphone correction on", Headphones),
  d("limiter", "Limiter", "Limiter on", ShieldCheck),
];

export interface EffectToggle extends EffectDescriptor {
  /** False for effects with no on/off state (Presets). */
  hasSwitch: boolean;
  enabled: boolean;
  /** False when the effect cannot be switched on from here (Headphone). */
  canToggleOn: boolean;
  toggle: (on: boolean) => void;
}

/** Whether `id` is on in `state`. Presets has no on/off state. */
export function enabledOf(id: EffectId, state: EngineState): boolean {
  switch (id) {
    case "equalizer":
      return state.eq.enabled;
    case "presets":
      return false;
    case "bass":
      return state.bass.enabled;
    case "surround":
      return state.spatializer.enabled;
    case "surround3d":
      return state.surround3d.enabled;
    case "room":
      return state.room.enabled;
    case "convolver":
      return state.convolver.enabled;
    case "compander":
      return state.compander.enabled;
    case "saturation":
      return state.saturation.enabled;
    case "script":
      return state.script.enabled;
    case "headphone":
      return state.headphone.enabled;
    case "limiter":
      return state.output.limiterEnabled;
  }
}

/**
 * Turn `id` on or off: the exact calls (and premium gates, which only guard
 * turning ON) the cards have always made. Reads the store when it runs, not
 * when a component rendered, so a toggle never sends a stale sibling value.
 */
export function toggleEffect(id: EffectId, on: boolean): void {
  const s = useEngineStore.getState();
  const st = s.state;
  switch (id) {
    case "equalizer":
      return s.setEqEnabled(on);
    case "presets":
      return;
    case "bass":
      return s.setBass(on, st.bass.amount, st.bass.harmonics, st.bass.adaptive);
    case "surround":
      return s.setSpatializer(on, st.spatializer.amount, st.spatializer.mode);
    case "surround3d":
      if (on && !requirePremium("SURROUND_3D")) return;
      return s.setSurround3d({ ...st.surround3d, enabled: on });
    case "room":
      if (on && !requirePremium("ROOM_REVERB")) return;
      return s.setRoom({ ...st.room, enabled: on });
    case "convolver":
      if (on && !requirePremium("CONVOLVER")) return;
      return s.setConvolver({ ...st.convolver, enabled: on });
    case "compander":
      return s.setCompander({ ...st.compander, enabled: on });
    case "saturation":
      return s.setSaturation({ ...st.saturation, enabled: on });
    case "script":
      return s.setScriptEnabled(on);
    case "headphone":
      // A profile must be chosen to turn this on; the rack jumps to the card.
      if (!on) s.clearProfile();
      return;
    case "limiter":
      return s.setOutput({ ...st.output, limiterEnabled: on });
  }
}

function toggleFor(descriptor: EffectDescriptor, enabled: boolean): EffectToggle {
  const { id } = descriptor;
  return {
    ...descriptor,
    hasSwitch: id !== "presets",
    enabled,
    canToggleOn: id !== "presets" && id !== "headphone",
    toggle: (on) => toggleEffect(id, on),
  };
}

const byId = (id: EffectId): EffectDescriptor =>
  EFFECTS.find((e) => e.id === id) as EffectDescriptor;

/**
 * One effect's on/off state and switch. The single source of truth for the
 * cards and the effects rack, so they can never disagree. Subscribes to that
 * effect's on/off flag only: dragging a slider elsewhere doesn't re-render it.
 */
export function useEffectToggle(id: EffectId): EffectToggle {
  const enabled = useEngineStore((s) => enabledOf(id, s.state));
  return useMemo(() => toggleFor(byId(id), enabled), [id, enabled]);
}

/** Every effect, for the rack. Re-renders only when an on/off flag flips. */
export function useEffectToggles(): Record<EffectId, EffectToggle> {
  const flags = useEngineStore(useShallow((s) => EFFECTS.map((e) => enabledOf(e.id, s.state))));
  return useMemo(
    () =>
      Object.fromEntries(EFFECTS.map((e, i) => [e.id, toggleFor(e, flags[i] ?? false)])) as Record<
        EffectId,
        EffectToggle
      >,
    [flags],
  );
}
