/** Format a frequency in Hz for compact axis labels (e.g. 1000 → "1k"). */
export function formatHz(hz: number): string {
  if (hz >= 1000) {
    const k = hz / 1000;
    return Number.isInteger(k) ? `${k}k` : `${k.toFixed(1)}k`;
  }
  return `${Math.round(hz)}`;
}

/** Format a dB gain with sign (e.g. +3.0, -6.0, 0.0). */
export function formatDb(db: number): string {
  const sign = db > 0 ? "+" : "";
  return `${sign}${db.toFixed(1)}`;
}
