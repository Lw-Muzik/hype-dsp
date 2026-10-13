# src/lib/eqBands.ts

- EqBandCount · type · L16-L16 — type EqBandCount = (typeof EQ_BAND_COUNTS)[number];
- bandFrequencies · function · L26-L33 — function bandFrequencies(n: number): number[]
- Anchor · interface · L35-L38 — interface Anchor
- sampleAt · function · L42-L60 — function sampleAt(anchors: Anchor[], hz: number): number
- sampleEngineAt · function · L63-L69 — function sampleEngineAt(bands31: readonly number[], freqs: number[]): number[]
- faderPointsToBands · function · L73-L80 — function faderPointsToBands(freqs: number[], gains: number[]): number[]
