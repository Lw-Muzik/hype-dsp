/** Renders a movie's length as `"1h 42m"` (or `"48m"` under an hour) — used
 *  wherever a YouTube movie's duration (known up front from channel
 *  discovery, unlike an Archive item) stands in for a year. Pure and
 *  DOM-free, same reasoning as `streamSelect.ts` sitting beside its player. */
export function formatDurationLabel(totalSec: number): string {
  if (!Number.isFinite(totalSec) || totalSec <= 0) return "";
  const totalMin = Math.round(totalSec / 60);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  if (h <= 0) return `${m}m`;
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}
