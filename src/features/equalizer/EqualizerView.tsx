import { FeatureView } from "@/components/FeatureView";
import { routeById } from "@/app/routes";

export function EqualizerView() {
  return <FeatureView route={routeById("equalizer")} />;
}
