/**
 * Deterministic gradient "cover art" for tracks/albums that have no embedded
 * artwork. A stable hash of the seed (album or title) picks two hues, so the
 * same album always gets the same vivid cover — like the abstract covers in
 * pro music UIs. (Real embedded-art extraction can replace this later.)
 */
function hash(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** A CSS `background` value for a cover, derived from `seed`. */
export function coverGradient(seed: string): string {
  const h = hash(seed || "untitled");
  const hue = h % 360;
  const hue2 = (hue + 55 + (h % 40)) % 360;
  return `linear-gradient(135deg, hsl(${hue} 72% 56%), hsl(${hue2} 76% 42%))`;
}

/** One or two uppercase initials for a cover overlay. */
export function coverInitials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "♪";
  if (words.length === 1) return words[0]!.slice(0, 2).toUpperCase();
  return (words[0]![0]! + words[1]![0]!).toUpperCase();
}
