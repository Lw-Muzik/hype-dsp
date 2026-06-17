import { FeatureView } from "@/components/FeatureView";
import { routeById } from "@/app/routes";

export function PlayerView() {
  return <FeatureView route={routeById("player")} />;
}
