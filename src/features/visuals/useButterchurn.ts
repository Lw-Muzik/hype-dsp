import { useCallback, useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import type { Butterchurn, ButterchurnVisualizer } from "butterchurn";
import {
  onVisualizerPcm,
  visualizerPcmStart,
  visualizerPcmStop,
} from "@/lib/ipc";

/** butterchurn's AudioProcessor uses numSamps 512 → fftSize 1024 time samples. */
const FFT_SIZE = 1024;

/**
 * Both butterchurn packages are UMD bundles, so depending on the interop the
 * real export can sit one or two `.default`s deep. Walk down until we find the
 * object that actually owns `method`.
 */
function unwrap<T>(mod: unknown, method: string): T | null {
  let cur: unknown = mod;
  for (let i = 0; i < 5 && cur != null; i++) {
    const obj = cur as Record<string, unknown>;
    if (typeof obj[method] === "function") return cur as T;
    cur = obj.default;
  }
  return null;
}

/** Fill butterchurn's three byte time-domain buffers from one mono PCM frame.
 *  The engine sends 512 samples in [-1, 1]; butterchurn wants 1024 bytes
 *  centered at 128, so each sample is written twice (a 2× nearest upsample). */
function fillBytes(
  frame: number[],
  mono: Uint8Array,
  left: Uint8Array,
  right: Uint8Array,
): void {
  const n = Math.min(frame.length, FFT_SIZE >> 1);
  for (let i = 0; i < n; i++) {
    let b = ((frame[i] ?? 0) * 128 + 128) | 0;
    b = b < 0 ? 0 : b > 255 ? 255 : b;
    const j = i << 1;
    mono[j] = b;
    mono[j + 1] = b;
    left[j] = b;
    left[j + 1] = b;
    right[j] = b;
    right[j + 1] = b;
  }
}

export interface UseButterchurn {
  /** True once the GL context + first preset are up. */
  ready: boolean;
  /** A human-readable failure (WebGL2 unsupported, import failed, …). */
  error: string | null;
  /** Every available preset name, sorted. */
  presetNames: string[];
  /** Cross-fade to a preset by name. */
  loadPreset: (name: string, blendSecs?: number) => void;
}

/**
 * Drives an embedded butterchurn (MilkDrop) visualizer on `canvasRef`, fed the
 * engine's post-DSP mono waveform over the `visualizer:pcm` event (no Web Audio
 * device — PCM is handed to `render({ audioLevels })` directly). Sets everything
 * up once on mount and tears it down on unmount.
 */
export function useButterchurn(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  initialPreset: string | null,
  onPresetLoaded: (name: string) => void,
): UseButterchurn {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [presetNames, setPresetNames] = useState<string[]>([]);

  const vizRef = useRef<ButterchurnVisualizer | null>(null);
  const presetsRef = useRef<Record<string, unknown>>({});
  // Keep the latest callback / initial choice without re-running the setup.
  const initialRef = useRef(initialPreset);
  const onLoadedRef = useRef(onPresetLoaded);
  onLoadedRef.current = onPresetLoaded;

  // Preallocated audio buffers, refilled in place each PCM frame. Start at 128
  // (centered = silence) so the visuals read as quiet, not a DC rail, before
  // the first frame arrives or while audio is paused.
  const monoRef = useRef(new Uint8Array(FFT_SIZE).fill(128));
  const leftRef = useRef(new Uint8Array(FFT_SIZE).fill(128));
  const rightRef = useRef(new Uint8Array(FFT_SIZE).fill(128));

  useEffect(() => {
    let disposed = false;
    let raf = 0;
    let unlisten: (() => void) | undefined;
    let audioCtx: AudioContext | undefined;
    let ro: ResizeObserver | undefined;

    void (async () => {
      const canvas = canvasRef.current;
      const parent = canvas?.parentElement;
      if (!canvas || !parent) return;
      try {
        // Load butterchurn plus the full preset library — the base pack only
        // has ~100; the Extra / Extra2 / MD1 packs bring the total to ~395.
        // Each is a lazy chunk, so this only downloads when the view opens.
        const [bcMod, base, extra, extra2, md1] = await Promise.all([
          import("butterchurn"),
          import("butterchurn-presets"),
          import("butterchurn-presets/lib/butterchurnPresetsExtra.min.js"),
          import("butterchurn-presets/lib/butterchurnPresetsExtra2.min.js"),
          import("butterchurn-presets/lib/butterchurnPresetsMD1.min.js"),
        ]);
        if (disposed) return;

        const butterchurn = unwrap<Butterchurn>(bcMod, "createVisualizer");
        if (!butterchurn) throw new Error("butterchurn failed to load");

        // Merge every pack (dedup by name — later packs win on collision).
        const presets: Record<string, unknown> = {};
        for (const mod of [base, extra, extra2, md1]) {
          const api = unwrap<{ getPresets(): Record<string, unknown> }>(
            mod,
            "getPresets",
          );
          if (api) Object.assign(presets, api.getPresets());
        }
        if (Object.keys(presets).length === 0) {
          throw new Error("preset packs failed to load");
        }
        presetsRef.current = presets;
        const names = Object.keys(presets).sort((a, b) =>
          a.toLowerCase().localeCompare(b.toLowerCase()),
        );

        const dpr = window.devicePixelRatio || 1;
        const w = Math.max(1, parent.clientWidth);
        const h = Math.max(1, parent.clientHeight);

        // We feed PCM by hand, so the context only supplies a sample rate and
        // the (unused) analyser plumbing — keep it suspended, never resumed.
        audioCtx = new AudioContext();
        void audioCtx.suspend().catch(() => {});

        const viz = butterchurn.createVisualizer(audioCtx, canvas, {
          width: w,
          height: h,
          pixelRatio: dpr,
          textureRatio: 1,
        });
        viz.setRendererSize(w, h);
        vizRef.current = viz;

        const chosen =
          initialRef.current && presets[initialRef.current]
            ? initialRef.current
            : names[0];
        if (chosen && presets[chosen]) {
          viz.loadPreset(presets[chosen], 0);
          onLoadedRef.current(chosen);
        }
        setPresetNames(names);

        ro = new ResizeObserver(() => {
          viz.setRendererSize(
            Math.max(1, parent.clientWidth),
            Math.max(1, parent.clientHeight),
          );
        });
        ro.observe(parent);

        // Start rendering immediately — the audio feed below is a best-effort
        // enhancement, so the visuals never depend on it succeeding.
        const render = () => {
          viz.render({
            audioLevels: {
              timeByteArray: monoRef.current,
              timeByteArrayL: leftRef.current,
              timeByteArrayR: rightRef.current,
            },
          });
          raf = requestAnimationFrame(render);
        };
        raf = requestAnimationFrame(render);
        setReady(true);
      } catch (e) {
        if (!disposed) {
          setError(
            e instanceof Error ? e.message : "Couldn't start the visualizer.",
          );
        }
        return;
      }

      // Feed the engine's PCM to the visuals. A failure here (e.g. running
      // outside Tauri) just means the presets animate on silence.
      try {
        await visualizerPcmStart();
        const fn = await onVisualizerPcm((frame) =>
          fillBytes(frame, monoRef.current, leftRef.current, rightRef.current),
        );
        if (disposed) fn();
        else unlisten = fn;
      } catch {
        // No audio reactivity available — visuals still run.
      }
    })();

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      ro?.disconnect();
      unlisten?.();
      void visualizerPcmStop().catch(() => {});
      vizRef.current = null;
      void audioCtx?.close().catch(() => {});
      // Explicitly drop the WebGL context so navigating in/out of the view many
      // times can't exhaust the browser's context pool (GPU memory leak).
      canvasRef.current
        ?.getContext("webgl2")
        ?.getExtension("WEBGL_lose_context")
        ?.loseContext();
    };
    // Set up once; latest callbacks/initial are read through refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadPreset = useCallback((name: string, blendSecs = 2.7) => {
    const viz = vizRef.current;
    const preset = presetsRef.current[name];
    if (viz && preset) viz.loadPreset(preset, blendSecs);
  }, []);

  return { ready, error, presetNames, loadPreset };
}
