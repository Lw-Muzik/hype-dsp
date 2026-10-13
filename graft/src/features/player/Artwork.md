# src/features/player/Artwork.tsx

- Artwork · function · L11-L57 — function Artwork({ art, seed, label, className, rounded = "rounded-lg", }: { /** Where to resolve real cover art from (omit for gradient-only). */ art?: ArtSource | null; /** Stable seed for the gradient fallback (album or title). */ seed: string; /** Source for the fallback initials (usually the title). */ label: string; className?: string; /** Tailwind rounding utility (so callers can match the surrounding shape). */ rounded?: string; })
