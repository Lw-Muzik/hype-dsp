import {
  AudioLines,
  Cast,
  Cloud,
  Info,
  Library,
  ListMusic,
  Palette,
  Sparkles,
  Speaker,
  SquarePlay,
  UserRound,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

/** Anchors in the Settings dialog (ids anchor `SettingsView`). */
export type SettingsSection =
  | "about"
  | "appearance"
  | "library"
  | "devices"
  | "cloud"
  | "youtube-music"
  | "playback"
  | "output"
  | "account"
  | "system-audio"
  | "visualizer";

/** The Settings dialog's sections, in page order. */
export const SETTINGS_SECTIONS: readonly { id: SettingsSection; label: string; icon: LucideIcon }[] = [
  { id: "about", label: "About", icon: Info },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "library", label: "Music library", icon: Library },
  { id: "devices", label: "Devices", icon: Cast },
  { id: "cloud", label: "Cloud", icon: Cloud },
  { id: "youtube-music", label: "YouTube Music", icon: SquarePlay },
  { id: "playback", label: "Playback", icon: ListMusic },
  { id: "output", label: "Output devices", icon: Speaker },
  { id: "account", label: "Account", icon: UserRound },
  { id: "system-audio", label: "System-wide audio", icon: Sparkles },
  { id: "visualizer", label: "Visualizer", icon: AudioLines },
];

export const isSettingsSection = (id: string): id is SettingsSection =>
  SETTINGS_SECTIONS.some((s) => s.id === id);
