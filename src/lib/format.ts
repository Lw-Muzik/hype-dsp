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

/** Format seconds as m:ss (or --:-- when unknown). */
export function formatTime(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "--:--";
  const total = Math.floor(secs);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Human-readable byte size, e.g. 4.2 MB. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
