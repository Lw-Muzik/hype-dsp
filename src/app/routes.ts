import {
  AudioLines,
  Compass,
  Gauge,
  Layers,
  LibraryBig,
  Settings,
  SlidersHorizontal,
  SlidersVertical,
  Tv,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { Route } from "@/stores/ui";

/** Static description of a navigable view, shared by the sidebar and headers. */
export interface NavRoute {
  id: Route;
  label: string;
  icon: LucideIcon;
  /** Sidebar grouping: primary destinations vs. system. */
  group: "main" | "system";
  /** One-line summary shown in the view header and empty state. */
  tagline: string;
  /** Hidden from the sidebar (reachable only inside another view, e.g. the
   * Player hub hosts Phone + Cloud as sources). */
  hidden?: boolean;
}

export const ROUTES: readonly NavRoute[] = [
  {
    id: "player",
    label: "Library",
    icon: LibraryBig,
    group: "main",
    tagline: "Local library and playlists, played through the enhancement chain.",
  },
  {
    id: "enhancer",
    label: "Enhancer",
    icon: Gauge,
    group: "main",
    tagline: "Power, master volume, surround and bass at a glance.",
  },
  {
    id: "equalizer",
    label: "Equalizer",
    icon: SlidersVertical,
    group: "main",
    tagline: "31-band graphic EQ with a live response curve over the spectrum.",
  },
  {
    id: "mixer",
    label: "Mixer",
    icon: SlidersHorizontal,
    group: "main",
    tagline: "Per-application volume and mute.",
  },
  {
    id: "stems",
    label: "Stems",
    icon: Layers,
    group: "main",
    tagline: "Split the track into vocals, drums, bass and instruments — then remix.",
  },
  {
    id: "explore",
    label: "Explore",
    icon: Compass,
    group: "main",
    tagline: "YouTube Music's own playlists and albums, browsed live.",
  },
  {
    id: "stations",
    label: "Stations",
    icon: Tv,
    group: "main",
    tagline: "Live radio and TV from around the world, streamed natively.",
  },
  {
    id: "visuals",
    label: "Visuals",
    icon: AudioLines,
    group: "main",
    tagline: "MilkDrop presets that dance to whatever's playing.",
  },
  {
    id: "settings",
    label: "Settings",
    icon: Settings,
    group: "system",
    tagline: "Audio devices, appearance, and about.",
  },
];

export const routeById = (id: Route): NavRoute => {
  const found = ROUTES.find((r) => r.id === id);
  // ROUTES is exhaustive over Route, so this is always defined.
  if (!found) throw new Error(`Unknown route: ${id}`);
  return found;
};
