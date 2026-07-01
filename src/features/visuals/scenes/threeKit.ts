import { useEffect } from "react";
import type { RefObject } from "react";
import * as THREE from "three";
import { MAX_DPR, useAudioData } from "./sceneKit";
import type { AudioSample } from "./sceneKit";

export { THREE };

/** Brand colours, reused across the 3D scenes. */
export const GOLD = new THREE.Color(0.96, 0.77, 0.27);
export const GREEN = new THREE.Color(0.29, 0.87, 0.5);

export interface ThreeCtx {
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  renderer: THREE.WebGLRenderer;
  width: number;
  height: number;
}

export interface ThreeScene {
  /** Called every animation frame before the render. */
  frame: (audio: AudioSample, dt: number) => void;
  /** Free any geometries/materials/textures the scene created. */
  dispose?: () => void;
}

/**
 * Boilerplate for a Three.js visualizer: a DPR-capped renderer, a perspective
 * camera, ResizeObserver sync, the audio feed, and a mounted-only rAF loop that
 * disposes everything on unmount. `build` sets the scene up once and returns its
 * per-frame update + cleanup.
 */
export function useThreeScene(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  build: (ctx: ThreeCtx) => ThreeScene,
): void {
  const { sample } = useAudioData();
  useEffect(() => {
    const canvas = canvasRef.current;
    const parent = canvas?.parentElement;
    if (!canvas || !parent) return;

    const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(MAX_DPR, window.devicePixelRatio || 1));
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(55, 1, 0.1, 400);

    const built = build({
      scene,
      camera,
      renderer,
      width: Math.max(1, parent.clientWidth),
      height: Math.max(1, parent.clientHeight),
    });

    const resize = () => {
      const w = Math.max(1, parent.clientWidth);
      const h = Math.max(1, parent.clientHeight);
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(parent);

    let raf = 0;
    let last = performance.now();
    const loop = () => {
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      built.frame(sample(dt), dt);
      renderer.render(scene, camera);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      built.dispose?.();
      // Force-release the WebGL context, not just its resources: `dispose()`
      // alone leaves the context slot reclaimable only by GC, so rapidly
      // switching 3D scenes (or StrictMode's mount→unmount→remount) can outrun
      // GC and hit the browser's hard ~16-context cap.
      renderer.forceContextLoss();
      renderer.dispose();
    };
    // build is captured once on mount (sample is stable).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sample]);
}
