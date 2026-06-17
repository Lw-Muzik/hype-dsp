import { FeatureView } from "@/components/FeatureView";
import { routeById } from "@/app/routes";

export function MixerView() {
  return <FeatureView route={routeById("mixer")} />;
}
