import { coverGradient } from "@/lib/cover";
import type { TrackMeta } from "@/lib/types";

export type BackdropSource =
  | { kind: "art"; url: string }
  | { kind: "gradient"; css: string };

/**
 * What the backdrop should paint for `meta`.
 *
 * `null` means paint nothing — the theme's plain surface shows. Note this is
 * only reached when nothing is playing: a *playing* track with no embedded art
 * gets the same deterministic gradient `Artwork` renders, so the backdrop and
 * the cover on screen always agree.
 */
export function backdropSource(meta: TrackMeta | null): BackdropSource | null {
  if (!meta) return null;
  if (meta.cover) return { kind: "art", url: meta.cover };
  return { kind: "gradient", css: coverGradient(meta.album || meta.title || "") };
}
