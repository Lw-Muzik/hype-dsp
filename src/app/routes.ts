import {
  Cloud,
  Music2,
  Radio,
  Settings,
  SlidersHorizontal,
  SlidersVertical,
  Smartphone,
  Sparkles,
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
}

export const ROUTES: readonly NavRoute[] = [
  {
    id: "enhancer",
    label: "Enhancer",
    icon: Sparkles,
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
    id: "player",
    label: "Player",
    icon: Music2,
    group: "main",
    tagline: "Local library and playlists, played through the enhancement chain.",
  },
  {
    id: "radio",
    label: "Radio",
    icon: Radio,
    group: "main",
    tagline: "Browse and stream internet radio through the same engine.",
  },
  {
    id: "cloud",
    label: "Cloud",
    icon: Cloud,
    group: "main",
    tagline: "Stream your music from Google Drive and Dropbox.",
  },
  {
    id: "phone",
    label: "Phone",
    icon: Smartphone,
    group: "main",
    tagline: "Stream the music on your phone, played through the desktop.",
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
