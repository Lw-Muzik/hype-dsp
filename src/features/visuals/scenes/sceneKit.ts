import { useCallback, useEffect, useRef } from "react";
import type { RefObject } from "react";
import { useEngineStore } from "@/stores/engine";
import { initialBeat, stepBeat } from "@/lib/beat";
import type { BeatState } from "@/lib/beat";

/** Cap the render resolution so high-DPI displays don't over-tax the GPU. */
export const MAX_DPR = 2;

/** One frame's worth of audio, derived from the engine's live meters/spectrum. */
export interface AudioSample {
  /** Overall loudness, 0..1. */
  level: number;
  /** Beat pulse, 0..1 — spikes on onsets, decays smoothly. */
  beat: number;
  /** Low / mid / high band energy, 0..1. */
  bass: number;
  mid: number;
  treble: number;
  /** Normalised spectrum bins, 0..1 (empty when idle). */
  spectrum: number[];
  /** Whether audio is actively playing. */
  playing: boolean;
}

function bandAvg(spectrum: number[], from: number, to: number): number {
  const a = Math.max(0, Math.floor(spectrum.length * from));
  const b = Math.min(spectrum.length, Math.ceil(spectrum.length * to));
  if (b <= a) return 0;
  let sum = 0;
  for (let i = a; i < b; i++) sum += spectrum[i] ?? 0;
  return sum / (b - a);
}

/**
 * Pulls live audio for a render frame. Returns a stable `sample(dt)` to call
 * once per animation frame — it reads the engine store transiently (no React
 * re-renders) and advances the beat envelope itself.
 */
export function useAudioData(): { sample: (dt: number) => AudioSample } {
  const beatL = useRef<BeatState>(initialBeat());
  const beatR = useRef<BeatState>(initialBeat());

  const sample = useCallback((dt: number): AudioSample => {
    const s = useEngineStore.getState();
    const live = s.metersLive && s.playing && !s.paused;
    const pL = live ? s.meters.peak[0] ?? 0 : 0;
    const pR = live ? s.meters.peak[1] ?? 0 : 0;
    const rL = live ? s.meters.rms[0] ?? 0 : 0;
    const rR = live ? s.meters.rms[1] ?? 0 : 0;
    const lvlL = Math.min(1, Math.max(rL * 1.25, pL));
    const lvlR = Math.min(1, Math.max(rR * 1.25, pR));
    beatL.current = stepBeat(beatL.current, lvlL, dt);
    beatR.current = stepBeat(beatR.current, lvlR, dt);
    const spectrum = live ? s.spectrum : [];
    return {
      level: (lvlL + lvlR) / 2,
      beat: Math.max(beatL.current.pulse, beatR.current.pulse),
      bass: bandAvg(spectrum, 0, 0.12),
      mid: bandAvg(spectrum, 0.12, 0.45),
      treble: bandAvg(spectrum, 0.45, 1),
      spectrum,
      playing: live,
    };
  }, []);

  return { sample };
}

/**
 * Keeps a canvas's drawing buffer sized to its parent (DPR-capped), so scenes
 * render crisp and fill the container at any window size. `onResize` fires after
 * each change with the device-pixel dimensions (e.g. to resize a WebGL view).
 */
export function useDprCanvas(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  onResize?: (w: number, h: number, dpr: number) => void,
): void {
  const cb = useRef(onResize);
  cb.current = onResize;
  useEffect(() => {
    const canvas = canvasRef.current;
    const parent = canvas?.parentElement;
    if (!canvas || !parent) return;
    const apply = () => {
      const dpr = Math.min(MAX_DPR, window.devicePixelRatio || 1);
      const w = Math.max(1, parent.clientWidth);
      const h = Math.max(1, parent.clientHeight);
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      cb.current?.(canvas.width, canvas.height, dpr);
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(parent);
    return () => ro.disconnect();
  }, [canvasRef]);
}
