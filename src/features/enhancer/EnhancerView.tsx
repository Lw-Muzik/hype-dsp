import { FeatureView } from "@/components/FeatureView";
import { routeById } from "@/app/routes";

export function EnhancerView() {
  return <FeatureView route={routeById("enhancer")} />;
}
