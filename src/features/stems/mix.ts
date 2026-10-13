/** The three live controls, in the engine's order (hm-audio::stems). */
export const STEMS = ["vocals", "drums", "instruments"] as const;
export type StemId = (typeof STEMS)[number];

export interface StemControl {
  /** Fader position, 0..1.5 (1 = the original level). */
  level: number;
  muted: boolean;
  solo: boolean;
}

export type StemControls = Record<StemId, StemControl>;

export const UNITY: StemControl = { level: 1, muted: false, solo: false };

export function initialControls(): StemControls {
  return { vocals: { ...UNITY }, drums: { ...UNITY }, instruments: { ...UNITY } };
}

/**
 * The gain the engine should apply to each control. Solo wins over mute: while
 * any control is soloed, every control that isn't is silent; otherwise a muted
 * control is silent and the rest follow their faders.
 */
export function effectiveGains(c: StemControls): [number, number, number] {
  const anySolo = STEMS.some((id) => c[id].solo);
  const gain = (id: StemId) => {
    const s = c[id];
    if (anySolo) return s.solo ? s.level : 0;
    return s.muted ? 0 : s.level;
  };
  return [gain("vocals"), gain("drums"), gain("instruments")];
}

/** Whether the mix currently equals the original track. */
export function isOriginal(c: StemControls): boolean {
  return effectiveGains(c).every((g) => g === 1);
}
