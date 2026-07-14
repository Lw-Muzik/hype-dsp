import type { FC } from "react";
import { useUiStore } from "@/stores/ui";
import type { Route } from "@/stores/ui";
import { EnhancerView } from "@/features/enhancer/EnhancerView";
import { EqualizerView } from "@/features/equalizer/EqualizerView";
import { MixerView } from "@/features/mixer/MixerView";
import { PlayerView } from "@/features/player/PlayerView";
import { SettingsView } from "@/features/settings/SettingsView";
import { StationsView } from "@/features/stations/StationsView";
import { StemsPanel } from "@/features/stems/StemsPanel";
import { VisualsView } from "@/features/visuals/VisualsView";

/**
 * Maps each route to its view. A typed `Record<Route, FC>` so adding a route to
 * the union forces a view here — there is no unhandled-route path.
 */
const VIEWS: Record<Route, FC> = {
  enhancer: EnhancerView,
  equalizer: EqualizerView,
  mixer: MixerView,
  stems: StemsPanel,
  player: PlayerView,
  stations: StationsView,
  visuals: VisualsView,
  settings: SettingsView,
};

/** Renders the active view from the UI store. */
export function Router() {
  const route = useUiStore((s) => s.route);
  const View = VIEWS[route];
  return <View />;
}
