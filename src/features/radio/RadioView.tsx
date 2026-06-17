import { FeatureView } from "@/components/FeatureView";
import { routeById } from "@/app/routes";

export function RadioView() {
  return <FeatureView route={routeById("radio")} />;
}
