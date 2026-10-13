import { coverGradient, coverInitials } from "@/lib/cover";

/** Icon-button look shared by the local and remote now-playing bars. */
export const barIconBtn =
  "flex size-8 items-center justify-center rounded-full text-text-muted transition-colors hover:text-text disabled:pointer-events-none disabled:opacity-30";

/** The current track's cover: art if there is any, else a gradient with initials. */
export function BarCover({ cover, seed }: { cover: string | null; seed: string }) {
  if (cover) {
    return (
      <img
        src={cover}
        alt=""
        className="size-12 shrink-0 rounded-lg object-cover shadow-sm"
        aria-hidden="true"
      />
    );
  }
  return (
    <div
      className="grid size-12 shrink-0 place-items-center rounded-lg text-sm font-semibold text-white/90 shadow-sm"
      style={{ background: coverGradient(seed) }}
      aria-hidden="true"
    >
      <span className="opacity-80">{coverInitials(seed)}</span>
    </div>
  );
}
