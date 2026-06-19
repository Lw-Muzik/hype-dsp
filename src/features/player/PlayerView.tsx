import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { MusicLibrary } from "@/features/player/MusicLibrary";

/**
 * The music hub: one unified library that merges the local library, paired
 * phones, and connected cloud accounts — browsable by Songs / Albums / Artists
 * / Folders / Genres with a global search across every source.
 */
export function PlayerView() {
  const route = routeById("player");
  return (
    <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />
      <MusicLibrary />
    </div>
  );
}
