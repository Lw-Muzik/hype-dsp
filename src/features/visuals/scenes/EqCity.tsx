import { useRef } from "react";
import { GOLD, GREEN, THREE, useThreeScene } from "./threeKit";

const GRID = 20; // GRID×GRID bars
const SPACING = 0.62;
const MAX_H = 4;

/** Equalizer City (3D) — a grid of bars extruding with the spectrum (radially,
 *  bass at the centre), with a ripple wave that rolls out on every beat. */
export function EqCity() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useThreeScene(canvasRef, ({ scene, camera }) => {
    const span = (GRID - 1) * SPACING;
    camera.position.set(span * 0.7, span * 0.75, span * 0.9);
    camera.lookAt(0, 0.5, 0);

    const geo = new THREE.BoxGeometry(0.42, 1, 0.42);
    const mat = new THREE.MeshStandardMaterial({
      roughness: 0.4,
      metalness: 0.1,
      emissiveIntensity: 0.6,
    });
    const mesh = new THREE.InstancedMesh(geo, mat, GRID * GRID);

    scene.add(new THREE.AmbientLight(0xffffff, 0.5));
    const key = new THREE.DirectionalLight(0xffffff, 0.8);
    key.position.set(4, 8, 5);
    scene.add(key);

    const m = new THREE.Matrix4();
    const col = new THREE.Color();
    const center = (GRID - 1) / 2;
    // Per-bar distance from centre (for the radial spectrum mapping + ripple).
    const dist: number[] = [];
    let maxDist = 0;
    for (let gx = 0; gx < GRID; gx++) {
      for (let gz = 0; gz < GRID; gz++) {
        const d = Math.hypot(gx - center, gz - center);
        dist[gx * GRID + gz] = d;
        if (d > maxDist) maxDist = d;
      }
    }

    const heights = new Float32Array(GRID * GRID);
    let ripple = 0; // expanding wavefront radius, grows after a beat
    let prevBeat = 0;

    return {
      frame: (a, dt) => {
        const spec = a.spectrum;
        if (a.beat > 0.5 && prevBeat <= 0.5) ripple = 0;
        prevBeat = a.beat;
        ripple += dt * maxDist * 1.6;

        for (let gx = 0; gx < GRID; gx++) {
          for (let gz = 0; gz < GRID; gz++) {
            const i = gx * GRID + gz;
            const d = dist[i] ?? 0;
            const idx = spec.length > 0 ? Math.floor((d / maxDist) * spec.length) : 0;
            const base = spec.length > 0 ? spec[idx] ?? 0 : 0;
            // Ripple: a soft bump where the wavefront currently is.
            const rip = Math.max(0, 1 - Math.abs(d - ripple) * 0.9) * a.beat;
            const target = (base + rip * 0.6) * MAX_H + 0.06;
            heights[i] = (heights[i] ?? 0) + (target - (heights[i] ?? 0)) * 0.4;
            const hgt = heights[i] ?? 0.06;

            m.makeScale(1, hgt, 1);
            m.setPosition(
              (gx - center) * SPACING,
              hgt / 2,
              (gz - center) * SPACING,
            );
            mesh.setMatrixAt(i, m);
            col.copy(GREEN).lerp(GOLD, Math.min(1, hgt / MAX_H));
            mesh.setColorAt(i, col);
          }
        }
        mesh.instanceMatrix.needsUpdate = true;
        if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
      },
      dispose: () => {
        geo.dispose();
        mat.dispose();
        mesh.dispose();
      },
    };
  });

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
